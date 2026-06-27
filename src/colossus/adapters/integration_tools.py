"""Tool adapters for connected integrations."""

import json
import re
from typing import Any

import httpx

from colossus.application.tools import ToolHandler
from colossus.domain.errors import ColossusError, ToolExecutionError
from colossus.domain.integrations import IntegrationConnection, IntegrationToolManifest
from colossus.domain.tools import ToolSpec
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.ports.audit import AuditSink
from colossus.ports.credentials import CredentialBroker

JsonObject = dict[str, object]


def create_integration_tools(
    connections: tuple[IntegrationConnection, ...],
    credential_broker: CredentialBroker,
    *,
    audit_sink: AuditSink | None = None,
    http_client_config: HttpClientConfig | None = None,
    http_transport: httpx.AsyncBaseTransport | None = None,
    github_api_base_url: str = "https://api.github.com",
) -> tuple[tuple[ToolSpec, ...], dict[str, ToolHandler]]:
    """Create tool specs and handlers for configured integration connections."""

    resolved_http = http_client_config or HttpClientConfig()
    specs: list[ToolSpec] = []
    handlers: dict[str, ToolHandler] = {}
    for connection in connections:
        if connection.status != "connected":
            continue
        for tool in connection.manifest.tools:
            specs.append(
                ToolSpec(
                    name=tool.name,
                    description=tool.description,
                    input_schema=tool.input_schema,
                    output_schema=tool.output_schema,
                    permissions=tool.permissions,
                    timeout_seconds=tool.timeout_seconds,
                    max_output_bytes=tool.max_output_bytes,
                )
            )
            handlers[tool.name] = _handler_for_tool(
                connection,
                tool,
                credential_broker,
                audit_sink=audit_sink,
                http_client_config=resolved_http,
                http_transport=http_transport,
                github_api_base_url=github_api_base_url,
            )
    return tuple(specs), handlers


def _handler_for_tool(
    connection: IntegrationConnection,
    tool: IntegrationToolManifest,
    credential_broker: CredentialBroker,
    *,
    audit_sink: AuditSink | None,
    http_client_config: HttpClientConfig,
    http_transport: httpx.AsyncBaseTransport | None,
    github_api_base_url: str,
) -> ToolHandler:
    async def handler(arguments: dict[str, object]) -> str:
        credential_value = _resolve_credential(connection, credential_broker)
        await _audit_tool_call(audit_sink, connection, tool, arguments)
        if connection.kind == "native" and connection.name == "github":
            return await _github_tool_call(
                tool.name,
                arguments,
                credential_value,
                http_client_config=http_client_config,
                http_transport=http_transport,
                base_url=github_api_base_url,
                timeout=tool.timeout_seconds,
            )
        if connection.kind == "openapi":
            return await _openapi_tool_call(
                connection,
                tool,
                arguments,
                credential_value,
                http_client_config=http_client_config,
                http_transport=http_transport,
                timeout=tool.timeout_seconds,
            )
        raise ToolExecutionError(f"Integration kind is not executable yet: {connection.kind}")

    return handler


def _resolve_credential(
    connection: IntegrationConnection,
    credential_broker: CredentialBroker,
) -> str | None:
    if connection.manifest.auth.type == "none":
        return None
    if not connection.credential_ref:
        raise ToolExecutionError(
            f"Authentication required for integration {connection.name}. "
            "Run `colossus integrations connect` with a credential ref."
        )
    try:
        return credential_broker.resolve(connection.credential_ref).value
    except ColossusError as exc:
        raise ToolExecutionError(str(exc)) from exc


async def _audit_tool_call(
    audit_sink: AuditSink | None,
    connection: IntegrationConnection,
    tool: IntegrationToolManifest,
    arguments: dict[str, object],
) -> None:
    if audit_sink is None:
        return
    await audit_sink.record(
        "tool",
        "integration.tool_called",
        {
            "integration": connection.name,
            "kind": connection.kind,
            "tool": tool.name,
            "credential_ref": connection.credential_ref or "",
            "argument_keys": sorted(arguments),
        },
    )


async def _github_tool_call(
    tool_name: str,
    arguments: dict[str, object],
    credential_value: str | None,
    *,
    http_client_config: HttpClientConfig,
    http_transport: httpx.AsyncBaseTransport | None,
    base_url: str,
    timeout: float,
) -> str:
    if credential_value is None:
        raise ToolExecutionError("GitHub integration requires a bearer token.")
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {credential_value}",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "colossus-agent/0.1",
    }
    path, params = _github_request(tool_name, arguments)
    response = await _request_json(
        "GET",
        _join_url(base_url, path),
        headers=headers,
        params=params,
        http_client_config=http_client_config,
        http_transport=http_transport,
        timeout=timeout,
    )
    return _json(response)


def _github_request(tool_name: str, arguments: dict[str, object]) -> tuple[str, JsonObject]:
    max_results = _int_arg(arguments, "max_results", 30, minimum=1, maximum=100)
    if tool_name == "github.repos":
        visibility = _str_arg(arguments, "visibility", default="all")
        return "/user/repos", {"visibility": visibility, "per_page": max_results}
    owner = _required_str(arguments, "owner")
    repo = _required_str(arguments, "repo")
    if tool_name == "github.issues":
        state = _str_arg(arguments, "state", default="open")
        return f"/repos/{owner}/{repo}/issues", {"state": state, "per_page": max_results}
    if tool_name == "github.pull_requests":
        state = _str_arg(arguments, "state", default="open")
        return f"/repos/{owner}/{repo}/pulls", {"state": state, "per_page": max_results}
    if tool_name == "github.checks":
        ref = _required_str(arguments, "ref")
        return f"/repos/{owner}/{repo}/commits/{ref}/check-runs", {"per_page": max_results}
    if tool_name == "github.releases":
        return f"/repos/{owner}/{repo}/releases", {"per_page": max_results}
    raise ToolExecutionError(f"Unsupported GitHub integration tool: {tool_name}")


async def _openapi_tool_call(
    connection: IntegrationConnection,
    tool: IntegrationToolManifest,
    arguments: dict[str, object],
    credential_value: str | None,
    *,
    http_client_config: HttpClientConfig,
    http_transport: httpx.AsyncBaseTransport | None,
    timeout: float,
) -> str:
    if not tool.method or not tool.path:
        raise ToolExecutionError(f"OpenAPI tool is missing operation metadata: {tool.name}")
    base_url = str(connection.manifest.metadata.get("base_url") or "")
    if not base_url:
        raise ToolExecutionError(f"OpenAPI integration is missing a base URL: {connection.name}")
    path, remaining = _substitute_path_args(tool.path, arguments)
    body = remaining.pop("body", None)
    method = tool.method.upper()
    params = remaining if method in {"GET", "DELETE"} else {}
    json_body = body if method not in {"GET", "DELETE"} else None
    response = await _request_json(
        method,
        _join_url(base_url, path),
        headers=_integration_auth_headers(connection, credential_value),
        params=params,
        json_body=json_body,
        http_client_config=http_client_config,
        http_transport=http_transport,
        timeout=timeout,
    )
    return _json(response)


def _integration_auth_headers(
    connection: IntegrationConnection,
    credential_value: str | None,
) -> dict[str, str]:
    auth = connection.manifest.auth
    if auth.type == "none":
        return {"User-Agent": "colossus-agent/0.1"}
    if credential_value is None:
        raise ToolExecutionError(f"Integration {connection.name} requires credentials.")
    if auth.type in {"bearer", "oauth2_authorization_code"}:
        value = f"{auth.scheme or 'Bearer'} {credential_value}".strip()
    elif auth.type == "api_key":
        value = credential_value if auth.scheme is None else f"{auth.scheme} {credential_value}"
    elif auth.type == "service_account":
        value = credential_value
    else:
        raise ToolExecutionError(f"Unsupported integration auth type: {auth.type}")
    return {
        "Accept": "application/json",
        auth.header: value,
        "User-Agent": "colossus-agent/0.1",
    }


async def _request_json(
    method: str,
    url: str,
    *,
    headers: dict[str, str],
    params: dict[str, object] | None = None,
    json_body: object | None = None,
    http_client_config: HttpClientConfig,
    http_transport: httpx.AsyncBaseTransport | None,
    timeout: float,
) -> JsonObject:
    try:
        async with httpx.AsyncClient(
            **http_client_config.async_client_kwargs(
                timeout=timeout,
                follow_redirects=True,
                transport=http_transport,
            )
        ) as client:
            response = await client.request(
                method,
                url,
                headers=headers,
                params=_http_params(params),
                json=json_body,
            )
            response.raise_for_status()
    except httpx.HTTPStatusError as exc:
        raise ToolExecutionError(
            f"Integration HTTP request returned {exc.response.status_code}."
        ) from exc
    except httpx.RequestError as exc:
        raise ToolExecutionError(
            f"Integration HTTP request failed: {exc.__class__.__name__}."
        ) from exc
    result: object
    try:
        result = response.json()
    except ValueError:
        result = response.text
    return {
        "status_code": response.status_code,
        "content_type": response.headers.get("content-type", ""),
        "result": _jsonable(result),
    }


def _substitute_path_args(
    path: str,
    arguments: dict[str, object],
) -> tuple[str, dict[str, object]]:
    remaining = dict(arguments)

    def replace(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in remaining:
            raise ToolExecutionError(f"Missing path argument: {key}")
        return str(remaining.pop(key))

    return re.sub(r"\{([^{}]+)\}", replace, path), remaining


def _join_url(base_url: str, path: str) -> str:
    return f"{base_url.rstrip('/')}/{path.lstrip('/')}"


def _http_params(
    params: dict[str, object] | None,
) -> dict[str, str | int | float | bool | None] | None:
    if params is None:
        return None
    clean: dict[str, str | int | float | bool | None] = {}
    for key, value in params.items():
        if isinstance(value, str | int | float | bool) or value is None:
            clean[key] = value
        else:
            clean[key] = json.dumps(_jsonable(value), sort_keys=True)
    return clean


def _required_str(arguments: dict[str, object], name: str) -> str:
    value = arguments.get(name)
    if not isinstance(value, str) or not value.strip():
        raise ToolExecutionError(f"Argument {name} must be a non-empty string.")
    return value.strip()


def _str_arg(arguments: dict[str, object], name: str, *, default: str) -> str:
    value = arguments.get(name, default)
    if not isinstance(value, str) or not value.strip():
        return default
    return value.strip()


def _int_arg(
    arguments: dict[str, object],
    name: str,
    default: int,
    *,
    minimum: int,
    maximum: int,
) -> int:
    value = arguments.get(name, default)
    if isinstance(value, bool) or not isinstance(value, int):
        return default
    return min(max(value, minimum), maximum)


def _jsonable(value: Any) -> object:
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_jsonable(item) for item in value]
    if isinstance(value, str | int | float | bool) or value is None:
        return value
    return str(value)


def _json(value: JsonObject) -> str:
    return json.dumps(value, sort_keys=True)
