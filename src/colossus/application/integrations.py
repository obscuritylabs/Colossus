"""Integration registry and connection management."""

import json
import re
from pathlib import Path
from typing import cast

from colossus.application.packs import PackService
from colossus.domain.errors import ColossusError
from colossus.domain.integrations import (
    IntegrationAuthRequirement,
    IntegrationAuthType,
    IntegrationConnection,
    IntegrationConnectionStatus,
    IntegrationManifest,
    IntegrationStatusView,
    IntegrationToolManifest,
    utc_now_iso,
)
from colossus.domain.tools import ToolPermission
from colossus.ports.audit import AuditSink
from colossus.ports.credentials import CredentialBroker
from colossus.ports.state import StateStore

HTTP_METHODS = frozenset({"get", "post", "put", "patch", "delete"})


class IntegrationService:
    def __init__(
        self,
        state_store: StateStore,
        audit_sink: AuditSink,
        credential_broker: CredentialBroker,
        *,
        pack_service: PackService | None = None,
    ) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink
        self._credential_broker = credential_broker
        self._pack_service = pack_service

    async def list_statuses(self) -> tuple[IntegrationStatusView, ...]:
        connections = {connection.name: connection for connection in await self._connections()}
        manifests = await self._available_manifests()
        views = [
            _status_for_manifest(manifest, connections.get(manifest.name))
            for manifest in manifests.values()
        ]
        for connection in connections.values():
            if connection.name in manifests:
                continue
            views.append(_status_for_manifest(connection.manifest, connection))
        return tuple(sorted(views, key=lambda item: item.name))

    async def get_manifest(self, name: str) -> IntegrationManifest:
        normalized = _normalize_name(name)
        manifests = await self._available_manifests()
        if normalized in manifests:
            return manifests[normalized]
        connection = await self._state_store.get_integration_connection(normalized)
        if connection is None:
            raise ColossusError(f"Unknown integration: {name}")
        return connection.manifest

    async def get_connection(self, name: str) -> IntegrationConnection | None:
        return await self._state_store.get_integration_connection(_normalize_name(name))

    async def connected_connections(self) -> tuple[IntegrationConnection, ...]:
        return tuple(
            connection
            for connection in await self._connections()
            if connection.status == "connected"
        )

    async def connect(
        self,
        name: str,
        *,
        credential_ref: str | None = None,
        credential_refs: dict[str, str] | None = None,
        scopes: tuple[str, ...] = (),
        config: dict[str, object] | None = None,
    ) -> IntegrationConnection:
        manifest = await self.get_manifest(name)
        now = utc_now_iso()
        existing = await self._state_store.get_integration_connection(manifest.name)
        connection_config = _connection_config(
            manifest,
            existing.config if existing is not None else {},
            config or {},
        )
        named_credential_refs = _connection_credential_refs(
            existing.credential_refs if existing is not None else {},
            credential_refs or {},
        )
        if manifest.name == "opensearch":
            opensearch_auth_type = _opensearch_auth_type(connection_config.get("auth_type"))
            if credential_ref is not None and opensearch_auth_type != "bearer":
                raise ColossusError(
                    "OpenSearch --credential-ref is only valid with --auth-type bearer."
                )
            if credential_refs and opensearch_auth_type != "basic":
                raise ColossusError(
                    "OpenSearch --username-ref and --password-ref are only valid "
                    "with --auth-type basic."
                )
            if opensearch_auth_type != "basic":
                named_credential_refs = {}
        status = _connection_status(
            manifest,
            credential_ref=credential_ref,
            credential_refs=named_credential_refs,
            config=connection_config,
        )
        if credential_ref is not None:
            self._credential_broker.resolve(credential_ref)
        for ref in named_credential_refs.values():
            self._credential_broker.resolve(ref)
        connection = IntegrationConnection(
            name=manifest.name,
            kind=manifest.kind,
            status=status,
            credential_ref=credential_ref,
            credential_refs=named_credential_refs,
            scopes=scopes or manifest.auth.scopes,
            manifest=manifest,
            config=connection_config,
            connected_at=existing.connected_at if existing is not None else now,
            updated_at=now,
        )
        await self._state_store.save_integration_connection(connection)
        await self._audit_sink.record(
            "user",
            "integration.connected" if status == "connected" else "integration.pending_auth",
            _audit_connection_details(connection),
        )
        return connection

    async def disconnect(self, name: str) -> None:
        normalized = _normalize_name(name)
        existing = await self._state_store.get_integration_connection(normalized)
        if existing is None:
            raise ColossusError(f"Integration is not connected: {name}")
        await self._state_store.delete_integration_connection(normalized)
        await self._audit_sink.record(
            "user",
            "integration.disconnected",
            {
                "name": existing.name,
                "kind": existing.kind,
                "status": existing.status,
            },
        )

    async def import_openapi(
        self,
        name: str,
        *,
        spec_path: Path,
        base_url: str | None = None,
        credential_ref: str | None = None,
        auth_type: IntegrationAuthType = "bearer",
    ) -> IntegrationConnection:
        manifest = openapi_manifest_from_file(
            name,
            spec_path=spec_path,
            base_url=base_url,
            auth_type=auth_type,
        )
        status: IntegrationConnectionStatus = "connected"
        if manifest.auth.type != "none":
            if credential_ref is None:
                status = "pending_auth"
            else:
                self._credential_broker.resolve(credential_ref)
        now = utc_now_iso()
        connection = IntegrationConnection(
            name=manifest.name,
            kind=manifest.kind,
            status=status,
            credential_ref=credential_ref,
            scopes=manifest.auth.scopes,
            manifest=manifest,
            config={
                "spec_path": str(spec_path.resolve()),
                "base_url": manifest.metadata.get("base_url", ""),
            },
            connected_at=now,
            updated_at=now,
        )
        await self._state_store.save_integration_connection(connection)
        await self._audit_sink.record(
            "user",
            "integration.openapi_imported",
            _audit_connection_details(connection),
        )
        return connection

    async def _connections(self) -> tuple[IntegrationConnection, ...]:
        return await self._state_store.list_integration_connections()

    async def _available_manifests(self) -> dict[str, IntegrationManifest]:
        manifests = dict(BUILTIN_MANIFESTS)
        if self._pack_service is not None:
            for manifest in await self._pack_service.integration_manifests():
                manifests.setdefault(manifest.name, manifest)
        return manifests


def openapi_manifest_from_file(
    name: str,
    *,
    spec_path: Path,
    base_url: str | None = None,
    auth_type: IntegrationAuthType = "bearer",
) -> IntegrationManifest:
    if not spec_path.exists():
        raise ColossusError(f"OpenAPI spec does not exist: {spec_path}")
    if not spec_path.is_file():
        raise ColossusError(f"OpenAPI spec is not a file: {spec_path}")
    try:
        data = json.loads(spec_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ColossusError(f"OpenAPI spec must be JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise ColossusError("OpenAPI spec must contain a JSON object.")
    return openapi_manifest_from_mapping(
        name,
        data,
        base_url=base_url,
        auth_type=auth_type,
    )


def openapi_manifest_from_mapping(
    name: str,
    data: dict[str, object],
    *,
    base_url: str | None = None,
    auth_type: IntegrationAuthType = "bearer",
) -> IntegrationManifest:
    normalized = _normalize_name(name)
    info = _mapping(data.get("info"))
    paths = _mapping(data.get("paths"))
    title = str(info.get("title") or normalized)
    description = str(info.get("description") or f"OpenAPI integration {normalized}.")
    resolved_base_url = base_url or _server_url(data)
    tools: list[IntegrationToolManifest] = []
    for raw_path, path_item in paths.items():
        if not isinstance(raw_path, str):
            continue
        operations = _mapping(path_item)
        for method, operation in operations.items():
            method_text = str(method).lower()
            if method_text not in HTTP_METHODS:
                continue
            operation_data = _mapping(operation)
            tools.append(
                _openapi_tool_manifest(
                    integration_name=normalized,
                    path=raw_path,
                    method=method_text,
                    operation=operation_data,
                )
            )
    if not tools:
        raise ColossusError("OpenAPI spec does not define supported operations.")
    auth = IntegrationAuthRequirement(type=auth_type)
    return IntegrationManifest(
        name=normalized,
        title=title,
        description=description,
        kind="openapi",
        auth=auth,
        tools=tuple(tools),
        metadata={"base_url": resolved_base_url},
    )


def _openapi_tool_manifest(
    *,
    integration_name: str,
    path: str,
    method: str,
    operation: dict[str, object],
) -> IntegrationToolManifest:
    operation_id = _operation_id(operation, method, path)
    properties: dict[str, object] = {}
    required: list[str] = []
    for parameter in _sequence(operation.get("parameters")):
        parameter_data = _mapping(parameter)
        parameter_name = parameter_data.get("name")
        location = parameter_data.get("in")
        if not isinstance(parameter_name, str) or location not in {"path", "query"}:
            continue
        schema = _simple_json_schema(_mapping(parameter_data.get("schema")))
        properties[parameter_name] = schema
        if parameter_data.get("required") is True or location == "path":
            required.append(parameter_name)
    request_body = _mapping(operation.get("requestBody"))
    if request_body:
        properties["body"] = {"type": "object"}
        if request_body.get("required") is True:
            required.append("body")
    input_schema: dict[str, object] = {
        "type": "object",
        "additionalProperties": False,
        "properties": properties,
    }
    if required:
        input_schema["required"] = required
    description = str(
        operation.get("description")
        or operation.get("summary")
        or f"{method.upper()} {path}"
    )
    return IntegrationToolManifest(
        name=f"openapi.{integration_name}.{_sanitize_tool_segment(operation_id)}",
        description=description,
        input_schema=input_schema,
        output_schema={"type": "object"},
        permissions=ToolPermission(
            network="allow",
            approval_required=True,
            working_root_required=False,
            risk="medium",
        ),
        max_output_bytes=64_000,
        operation_id=operation_id,
        method=method,
        path=path,
    )


def _status_for_manifest(
    manifest: IntegrationManifest,
    connection: IntegrationConnection | None,
) -> IntegrationStatusView:
    return IntegrationStatusView(
        name=manifest.name,
        title=manifest.title,
        kind=manifest.kind,
        status=connection.status if connection is not None else "available",
        auth_type=_status_auth_type(manifest, connection),
        credential_ref=connection.credential_ref if connection is not None else None,
        credential_refs=connection.credential_refs if connection is not None else {},
        scopes=connection.scopes if connection is not None else manifest.auth.scopes,
        tools=tuple(tool.name for tool in manifest.tools),
    )


def _audit_connection_details(connection: IntegrationConnection) -> dict[str, object]:
    return {
        "name": connection.name,
        "kind": connection.kind,
        "status": connection.status,
        "credential_ref": connection.credential_ref or "",
        "credential_refs": dict(sorted(connection.credential_refs.items())),
        "scopes": list(connection.scopes),
        "tools": [tool.name for tool in connection.manifest.tools],
        "config_keys": sorted(connection.config),
    }


def _status_auth_type(
    manifest: IntegrationManifest,
    connection: IntegrationConnection | None,
) -> IntegrationAuthType:
    if connection is not None and manifest.name == "opensearch":
        return _opensearch_auth_type(connection.config.get("auth_type"))
    return manifest.auth.type


def _connection_status(
    manifest: IntegrationManifest,
    *,
    credential_ref: str | None,
    credential_refs: dict[str, str],
    config: dict[str, object],
) -> IntegrationConnectionStatus:
    if manifest.name == "opensearch":
        auth_type = _opensearch_auth_type(config.get("auth_type"))
        if auth_type == "none":
            return "connected"
        if auth_type == "bearer":
            return "connected" if credential_ref else "pending_auth"
        if auth_type == "basic":
            return (
                "connected"
                if credential_refs.get("username") and credential_refs.get("password")
                else "pending_auth"
            )
    if credential_ref is not None:
        return "connected"
    if manifest.auth.type != "none":
        return "pending_auth"
    return "connected"


def _connection_credential_refs(
    existing: dict[str, str],
    updates: dict[str, str],
) -> dict[str, str]:
    result = {str(key): str(value) for key, value in existing.items() if str(value).strip()}
    for key, value in updates.items():
        normalized_key = str(key).strip().lower()
        normalized_value = str(value).strip()
        if normalized_key and normalized_value:
            result[normalized_key] = normalized_value
    return result


def _connection_config(
    manifest: IntegrationManifest,
    existing: dict[str, object],
    updates: dict[str, object],
) -> dict[str, object]:
    if manifest.name == "opensearch":
        return _opensearch_connection_config(manifest, existing, updates)
    if manifest.name != "searxng":
        unknown = sorted(
            key for key, value in updates.items() if value is not None and value != ""
        )
        if unknown:
            raise ColossusError(
                f"Integration {manifest.name} does not support config options: "
                f"{', '.join(unknown)}"
            )
        return existing
    allowed = {"base_url", "auth_header", "auth_scheme"}
    unknown = sorted(set(updates) - allowed)
    if unknown:
        raise ColossusError(
            f"Integration searxng does not support config options: {', '.join(unknown)}"
        )
    config: dict[str, object] = {
        "base_url": str(
            existing.get("base_url")
            or manifest.metadata.get("base_url")
            or "http://localhost:8888/search"
        ),
        "auth_header": str(
            existing.get("auth_header")
            or manifest.metadata.get("auth_header")
            or "Authorization"
        ),
        "auth_scheme": str(
            existing.get("auth_scheme")
            or manifest.metadata.get("auth_scheme")
            or "bearer"
        ),
    }
    for key in allowed:
        value = updates.get(key)
        if isinstance(value, str) and value.strip():
            config[key] = value.strip()
    if not str(config["base_url"]).strip():
        raise ColossusError("SearXNG base URL is required.")
    return config


def _opensearch_connection_config(
    manifest: IntegrationManifest,
    existing: dict[str, object],
    updates: dict[str, object],
) -> dict[str, object]:
    allowed = {"base_url", "auth_type", "auth_header", "auth_scheme"}
    unknown = sorted(set(updates) - allowed)
    if unknown:
        raise ColossusError(
            f"Integration opensearch does not support config options: {', '.join(unknown)}"
        )
    config: dict[str, object] = {
        "base_url": str(
            existing.get("base_url")
            or manifest.metadata.get("base_url")
            or "http://localhost:9200"
        ),
        "auth_type": _opensearch_auth_type(
            existing.get("auth_type") or manifest.metadata.get("auth_type") or "none"
        ),
        "auth_header": str(
            existing.get("auth_header")
            or manifest.metadata.get("auth_header")
            or "Authorization"
        ),
        "auth_scheme": str(
            existing.get("auth_scheme")
            or manifest.metadata.get("auth_scheme")
            or "Bearer"
        ),
    }
    for key in {"base_url", "auth_header", "auth_scheme"}:
        value = updates.get(key)
        if isinstance(value, str) and value.strip():
            config[key] = value.strip()
    auth_type_update = updates.get("auth_type")
    if isinstance(auth_type_update, str) and auth_type_update.strip():
        config["auth_type"] = _opensearch_auth_type(auth_type_update)
    if not str(config["base_url"]).strip():
        raise ColossusError("OpenSearch base URL is required.")
    return config


def _opensearch_auth_type(value: object) -> IntegrationAuthType:
    normalized = str(value or "none").strip().lower().replace("-", "_")
    if normalized not in {"none", "basic", "bearer"}:
        raise ColossusError("OpenSearch auth type must be none, basic, or bearer.")
    return cast(IntegrationAuthType, normalized)


def _normalize_name(name: str) -> str:
    normalized = name.strip().lower()
    if not normalized:
        raise ColossusError("Integration name is required.")
    if not re.fullmatch(r"[a-z0-9_.-]+", normalized):
        raise ColossusError(
            "Integration names may contain only letters, numbers, dots, underscores, and dashes."
        )
    return normalized


def _operation_id(operation: dict[str, object], method: str, path: str) -> str:
    raw = operation.get("operationId")
    if isinstance(raw, str) and raw.strip():
        return raw.strip()
    return f"{method}_{path.strip('/').replace('/', '_').replace('{', '').replace('}', '')}"


def _sanitize_tool_segment(value: str) -> str:
    return re.sub(r"[^a-zA-Z0-9_]", "_", value).strip("_").lower() or "operation"


def _simple_json_schema(schema: dict[str, object]) -> dict[str, object]:
    schema_type = schema.get("type")
    if schema_type in {"string", "integer", "number", "boolean"}:
        return {"type": schema_type}
    return {"type": "string"}


def _server_url(data: dict[str, object]) -> str:
    servers = _sequence(data.get("servers"))
    if servers:
        first = _mapping(servers[0])
        url = first.get("url")
        if isinstance(url, str):
            return url
    return ""


def _mapping(value: object) -> dict[str, object]:
    return value if isinstance(value, dict) else {}


def _sequence(value: object) -> list[object]:
    return value if isinstance(value, list) else []


SEARXNG_MANIFEST = IntegrationManifest(
    name="searxng",
    title="SearXNG",
    description=(
        "Native local/private metasearch connector for SearXNG JSON search results."
    ),
    kind="native",
    auth=IntegrationAuthRequirement(type="none"),
    tools=(
        IntegrationToolManifest(
            name="searxng.search",
            description="Search a configured SearXNG instance and return normalized results.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "query": {"type": "string"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 20},
                },
                "required": ["query"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            timeout_seconds=20.0,
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="searxng.health",
            description="Check that the configured SearXNG instance returns JSON results.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {},
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            timeout_seconds=20.0,
            max_output_bytes=8_000,
        ),
    ),
    metadata={
        "base_url": "http://localhost:8888/search",
        "auth_header": "Authorization",
        "auth_scheme": "bearer",
    },
)


OPENSEARCH_MANIFEST = IntegrationManifest(
    name="opensearch",
    title="OpenSearch",
    description=(
        "Native OpenSearch connector for document search, retrieval, indexing, "
        "updates, deletes, mappings, and cluster health."
    ),
    kind="native",
    auth=IntegrationAuthRequirement(type="none"),
    tools=(
        IntegrationToolManifest(
            name="opensearch.info",
            description="Fetch basic information from the configured OpenSearch endpoint.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {},
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=16_000,
        ),
        IntegrationToolManifest(
            name="opensearch.health",
            description="Fetch OpenSearch cluster health from _cluster/health.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {},
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=16_000,
        ),
        IntegrationToolManifest(
            name="opensearch.list_indices",
            description="List OpenSearch indices through the JSON cat indices API.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {},
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="opensearch.get_mapping",
            description="Fetch the mapping for an OpenSearch index or index pattern.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {"index": {"type": "string"}},
                "required": ["index"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="opensearch.search",
            description="Run an OpenSearch query object against an index or index pattern.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "index": {"type": "string"},
                    "query": {"type": "object"},
                    "size": {"type": "integer", "minimum": 1, "maximum": 100},
                    "from": {"type": "integer", "minimum": 0},
                    "source_includes": {
                        "type": "array",
                        "items": {"type": "string"},
                    },
                    "sort": {"type": "array", "items": {"type": "object"}},
                },
                "required": ["index", "query"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=128_000,
        ),
        IntegrationToolManifest(
            name="opensearch.get_document",
            description="Fetch one OpenSearch document by index and id.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "index": {"type": "string"},
                    "id": {"type": "string"},
                },
                "required": ["index", "id"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="opensearch.index_document",
            description="Create or replace one OpenSearch document.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "index": {"type": "string"},
                    "id": {"type": "string"},
                    "document": {"type": "object"},
                    "refresh": {"type": "string", "enum": ["false", "true", "wait_for"]},
                },
                "required": ["index", "document"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                mutation=True,
                working_root_required=False,
                risk="high",
            ),
            max_output_bytes=32_000,
        ),
        IntegrationToolManifest(
            name="opensearch.update_document",
            description="Partially update one OpenSearch document using a doc payload.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "index": {"type": "string"},
                    "id": {"type": "string"},
                    "doc": {"type": "object"},
                    "doc_as_upsert": {"type": "boolean"},
                    "refresh": {"type": "string", "enum": ["false", "true", "wait_for"]},
                },
                "required": ["index", "id", "doc"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                mutation=True,
                working_root_required=False,
                risk="high",
            ),
            max_output_bytes=32_000,
        ),
        IntegrationToolManifest(
            name="opensearch.delete_document",
            description="Delete one OpenSearch document by index and id.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "index": {"type": "string"},
                    "id": {"type": "string"},
                    "refresh": {"type": "string", "enum": ["false", "true", "wait_for"]},
                },
                "required": ["index", "id"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                mutation=True,
                working_root_required=False,
                risk="high",
            ),
            max_output_bytes=32_000,
        ),
    ),
    metadata={
        "base_url": "http://localhost:9200",
        "auth_type": "none",
        "auth_header": "Authorization",
        "auth_scheme": "Bearer",
    },
)


GITHUB_MANIFEST = IntegrationManifest(
    name="github",
    title="GitHub",
    description=(
        "Native GitHub connector for repositories, issues, pull requests, checks, "
        "and releases."
    ),
    kind="native",
    auth=IntegrationAuthRequirement(
        type="bearer",
        scopes=("repo", "workflow"),
    ),
    tools=(
        IntegrationToolManifest(
            name="github.repos",
            description="List repositories visible to the connected GitHub token.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "visibility": {
                        "type": "string",
                        "enum": ["all", "public", "private"],
                        "default": "all",
                    },
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100},
                },
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="github.issues",
            description="List issues for a GitHub repository.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "state": {"type": "string", "enum": ["open", "closed", "all"]},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100},
                },
                "required": ["owner", "repo"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="github.pull_requests",
            description="List pull requests for a GitHub repository.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "state": {"type": "string", "enum": ["open", "closed", "all"]},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100},
                },
                "required": ["owner", "repo"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="github.checks",
            description="List check runs for a GitHub commit ref.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "ref": {"type": "string"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100},
                },
                "required": ["owner", "repo", "ref"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
        IntegrationToolManifest(
            name="github.releases",
            description="List releases for a GitHub repository.",
            input_schema={
                "type": "object",
                "additionalProperties": False,
                "properties": {
                    "owner": {"type": "string"},
                    "repo": {"type": "string"},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 100},
                },
                "required": ["owner", "repo"],
            },
            output_schema={"type": "object"},
            permissions=ToolPermission(
                network="allow",
                approval_required=True,
                working_root_required=False,
                risk="medium",
            ),
            max_output_bytes=64_000,
        ),
    ),
)

BUILTIN_MANIFESTS = {
    "github": GITHUB_MANIFEST,
    "opensearch": OPENSEARCH_MANIFEST,
    "searxng": SEARXNG_MANIFEST,
}
