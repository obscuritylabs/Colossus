"""Tool adapters for connected integrations."""

import base64
import json
import re
from typing import Any
from urllib.parse import quote

import httpx

from colossus.adapters.research_sources import SearxngSearchProvider
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
        credential_values = _resolve_named_credentials(connection, credential_broker)
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
        if connection.kind == "native" and connection.name == "searxng":
            return await _searxng_tool_call(
                connection,
                tool.name,
                arguments,
                credential_value,
                http_client_config=http_client_config,
                http_transport=http_transport,
            )
        if connection.kind == "native" and connection.name == "opensearch":
            return await _opensearch_tool_call(
                connection,
                tool.name,
                arguments,
                credential_value,
                credential_values,
                http_client_config=http_client_config,
                http_transport=http_transport,
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
    if connection.credential_ref:
        try:
            return credential_broker.resolve(connection.credential_ref).value
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
    if connection.manifest.auth.type == "none":
        return None
    raise ToolExecutionError(
        f"Authentication required for integration {connection.name}. "
        "Run `colossus integrations connect` with a credential ref."
    )


def _resolve_named_credentials(
    connection: IntegrationConnection,
    credential_broker: CredentialBroker,
) -> dict[str, str]:
    values: dict[str, str] = {}
    for name, ref in connection.credential_refs.items():
        try:
            values[name] = credential_broker.resolve(ref).value
        except ColossusError as exc:
            raise ToolExecutionError(str(exc)) from exc
    return values


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
            "credential_refs": dict(sorted(connection.credential_refs.items())),
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


async def _searxng_tool_call(
    connection: IntegrationConnection,
    tool_name: str,
    arguments: dict[str, object],
    credential_value: str | None,
    *,
    http_client_config: HttpClientConfig,
    http_transport: httpx.AsyncBaseTransport | None,
) -> str:
    provider = SearxngSearchProvider(
        endpoint=_searxng_config(connection, "base_url", "http://localhost:8888/search"),
        api_key=credential_value,
        auth_header=_searxng_config(connection, "auth_header", "Authorization"),
        auth_scheme=_searxng_config(connection, "auth_scheme", "bearer").lower(),
        transport=http_transport,
        http_client_config=http_client_config,
    )
    if tool_name == "searxng.search":
        query = _required_str(arguments, "query")
        max_results = _int_arg(arguments, "max_results", 10, minimum=1, maximum=20)
        drafts = await _collect_searxng(provider, query, max_results=max_results)
        return _json(
            {
                "query": query,
                "count": len(drafts),
                "results": [_source_draft_json(draft) for draft in drafts],
            }
        )
    if tool_name == "searxng.health":
        drafts = await _collect_searxng(provider, "colossus", max_results=1)
        return _json({"status": "ok", "result_count": len(drafts)})
    raise ToolExecutionError(f"Unsupported SearXNG integration tool: {tool_name}")


async def _collect_searxng(
    provider: SearxngSearchProvider,
    query: str,
    *,
    max_results: int,
) -> tuple[object, ...]:
    try:
        return await provider.collect(query, max_results=max_results)
    except httpx.HTTPStatusError as exc:
        raise ToolExecutionError(
            f"SearXNG integration request returned {exc.response.status_code}."
        ) from exc
    except httpx.RequestError as exc:
        raise ToolExecutionError(
            f"SearXNG integration request failed: {exc.__class__.__name__}."
        ) from exc


def _source_draft_json(draft: object) -> JsonObject:
    return {
        "title": str(getattr(draft, "title", "")),
        "url": str(getattr(draft, "uri", "")),
        "content": str(getattr(draft, "content", "")),
        "metadata": _jsonable(getattr(draft, "metadata", {})),
    }


def _searxng_config(connection: IntegrationConnection, key: str, default: str) -> str:
    value = connection.config.get(key) or connection.manifest.metadata.get(key) or default
    return str(value)


async def _opensearch_tool_call(
    connection: IntegrationConnection,
    tool_name: str,
    arguments: dict[str, object],
    credential_value: str | None,
    credential_values: dict[str, str],
    *,
    http_client_config: HttpClientConfig,
    http_transport: httpx.AsyncBaseTransport | None,
    timeout: float,
) -> str:
    method, path, params, body = _opensearch_request(tool_name, arguments)
    response = await _request_json(
        method,
        _join_url(_opensearch_config(connection, "base_url", "http://localhost:9200"), path),
        headers=_opensearch_auth_headers(connection, credential_value, credential_values),
        params=params,
        json_body=body,
        http_client_config=http_client_config,
        http_transport=http_transport,
        timeout=timeout,
    )
    return _json(response)


def _opensearch_request(
    tool_name: str,
    arguments: dict[str, object],
) -> tuple[str, str, JsonObject, object | None]:
    if tool_name == "opensearch.info":
        return "GET", "/", {}, None
    if tool_name == "opensearch.health":
        return "GET", "/_cluster/health", {}, None
    if tool_name == "opensearch.list_indices":
        return "GET", "/_cat/indices", {"format": "json"}, None
    if tool_name == "opensearch.get_mapping":
        index = _opensearch_index(arguments)
        return "GET", f"/{index}/_mapping", {}, None
    if tool_name == "opensearch.search":
        index = _opensearch_index(arguments)
        search_body: JsonObject = {
            "query": _object_arg(arguments, "query"),
            "size": _int_arg(arguments, "size", 10, minimum=1, maximum=100),
            "from": _int_arg(arguments, "from", 0, minimum=0, maximum=10_000),
        }
        source_includes = _string_list_arg(arguments, "source_includes")
        if source_includes:
            search_body["_source"] = source_includes
        sort = _object_list_arg(arguments, "sort")
        if sort:
            search_body["sort"] = sort
        return "POST", f"/{index}/_search", {}, search_body
    if tool_name == "opensearch.get_document":
        index = _opensearch_index(arguments)
        document_id = _opensearch_id(arguments)
        return "GET", f"/{index}/_doc/{document_id}", {}, None
    if tool_name == "opensearch.index_document":
        index = _opensearch_index(arguments)
        document = _object_arg(arguments, "document")
        params = _refresh_params(arguments)
        raw_id = arguments.get("id")
        if isinstance(raw_id, str) and raw_id.strip():
            return "PUT", f"/{index}/_doc/{_path_segment(raw_id)}", params, document
        return "POST", f"/{index}/_doc", params, document
    if tool_name == "opensearch.update_document":
        index = _opensearch_index(arguments)
        document_id = _opensearch_id(arguments)
        update_body: JsonObject = {"doc": _object_arg(arguments, "doc")}
        doc_as_upsert = arguments.get("doc_as_upsert")
        if isinstance(doc_as_upsert, bool):
            update_body["doc_as_upsert"] = doc_as_upsert
        return "POST", f"/{index}/_update/{document_id}", _refresh_params(arguments), update_body
    if tool_name == "opensearch.delete_document":
        index = _opensearch_index(arguments)
        document_id = _opensearch_id(arguments)
        return "DELETE", f"/{index}/_doc/{document_id}", _refresh_params(arguments), None
    raise ToolExecutionError(f"Unsupported OpenSearch integration tool: {tool_name}")


def _opensearch_auth_headers(
    connection: IntegrationConnection,
    credential_value: str | None,
    credential_values: dict[str, str],
) -> dict[str, str]:
    headers = {
        "Accept": "application/json",
        "User-Agent": "colossus-agent/0.1",
    }
    auth_type = _opensearch_auth_type(connection.config.get("auth_type"))
    if auth_type == "none":
        return headers
    auth_header = _opensearch_config(connection, "auth_header", "Authorization")
    if auth_type == "bearer":
        if credential_value is None:
            raise ToolExecutionError("OpenSearch bearer/proxy auth requires a credential ref.")
        auth_scheme = _opensearch_config(connection, "auth_scheme", "Bearer")
        headers[auth_header] = (
            credential_value
            if auth_scheme.lower() == "raw"
            else f"{auth_scheme} {credential_value}".strip()
        )
        return headers
    if auth_type == "basic":
        username = credential_values.get("username")
        password = credential_values.get("password")
        if username is None or password is None:
            raise ToolExecutionError(
                "OpenSearch basic auth requires username and password credential refs."
            )
        token = base64.b64encode(f"{username}:{password}".encode()).decode("ascii")
        headers[auth_header] = f"Basic {token}"
        return headers
    raise ToolExecutionError(f"Unsupported OpenSearch auth type: {auth_type}")


def _opensearch_config(connection: IntegrationConnection, key: str, default: str) -> str:
    value = connection.config.get(key) or connection.manifest.metadata.get(key) or default
    return str(value)


def _opensearch_auth_type(value: object) -> str:
    normalized = str(value or "none").strip().lower().replace("-", "_")
    if normalized in {"none", "basic", "bearer"}:
        return normalized
    raise ToolExecutionError("OpenSearch auth type must be none, basic, or bearer.")


def _opensearch_index(arguments: dict[str, object]) -> str:
    return _path_segment(_required_str(arguments, "index"), safe=",*")


def _opensearch_id(arguments: dict[str, object]) -> str:
    return _path_segment(_required_str(arguments, "id"))


def _path_segment(value: str, *, safe: str = "") -> str:
    return quote(value.strip(), safe=safe)


def _refresh_params(arguments: dict[str, object]) -> JsonObject:
    value = arguments.get("refresh")
    if isinstance(value, str) and value in {"false", "true", "wait_for"}:
        return {"refresh": value}
    return {}


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


def _object_arg(arguments: dict[str, object], name: str) -> JsonObject:
    value = arguments.get(name)
    if not isinstance(value, dict):
        raise ToolExecutionError(f"Argument {name} must be an object.")
    return {str(key): _jsonable(item) for key, item in value.items()}


def _string_list_arg(arguments: dict[str, object], name: str) -> list[str]:
    value = arguments.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ToolExecutionError(f"Argument {name} must be an array of strings.")
        if item.strip():
            result.append(item.strip())
    return result


def _object_list_arg(arguments: dict[str, object], name: str) -> list[JsonObject]:
    value = arguments.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ToolExecutionError(f"Argument {name} must be an array of objects.")
    result: list[JsonObject] = []
    for item in value:
        if not isinstance(item, dict):
            raise ToolExecutionError(f"Argument {name} must be an array of objects.")
        result.append({str(key): _jsonable(entry) for key, entry in item.items()})
    return result


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
