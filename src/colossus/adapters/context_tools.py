"""Model-callable context management tools."""

import json
from typing import Any

from colossus.application.context import ContextService
from colossus.application.tools import ToolHandler
from colossus.domain.errors import ToolExecutionError
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.ports.model_provider import ModelProvider

JsonObject = dict[str, Any]
HandlerMap = dict[str, ToolHandler]


def create_context_tools(
    context_service: ContextService | None,
    *,
    provider: ModelProvider | None = None,
    default_model: str = "default",
) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    handlers = ContextToolHandlers(context_service, provider=provider, default_model=default_model)
    specs = (
        _context_show_spec(),
        _context_compact_spec(),
        _context_snapshots_spec(),
        _context_restore_spec(),
    )
    return specs, {
        "context.show": handlers.context_show,
        "context.compact": handlers.context_compact,
        "context.snapshots": handlers.context_snapshots,
        "context.restore": handlers.context_restore,
    }


class ContextToolHandlers:
    def __init__(
        self,
        context_service: ContextService | None,
        *,
        provider: ModelProvider | None,
        default_model: str,
    ) -> None:
        self._context_service = context_service
        self._provider = provider
        self._default_model = default_model

    async def context_show(self, arguments: JsonObject) -> str:
        service = self._require_service()
        status = await service.status(
            _required_string_arg(arguments, "session_id"),
            _string_arg(arguments, "model", self._default_model),
        )
        return _json({"status": status.model_dump(mode="json")})

    async def context_compact(self, arguments: JsonObject) -> str:
        service = self._require_service()
        snapshot = await service.compact_session(
            session_id=_required_string_arg(arguments, "session_id"),
            model=_string_arg(arguments, "model", self._default_model),
            provider=self._provider,
        )
        return _json({"snapshot": snapshot.model_dump(mode="json")})

    async def context_snapshots(self, arguments: JsonObject) -> str:
        service = self._require_service()
        snapshots = await service.list_snapshots(_required_string_arg(arguments, "session_id"))
        return _json({"snapshots": [snapshot.model_dump(mode="json") for snapshot in snapshots]})

    async def context_restore(self, arguments: JsonObject) -> str:
        service = self._require_service()
        snapshot = await service.restore_snapshot(_required_string_arg(arguments, "snapshot_id"))
        return _json({"restored": True, "snapshot": snapshot.model_dump(mode="json")})

    def _require_service(self) -> ContextService:
        if self._context_service is None:
            raise ToolExecutionError("Context service is not configured.")
        return self._context_service


def _required_string_arg(arguments: JsonObject, name: str) -> str:
    value = arguments.get(name)
    if not isinstance(value, str) or not value:
        raise ToolExecutionError(f"Argument {name} must be a non-empty string.")
    return value


def _string_arg(arguments: JsonObject, name: str, default: str) -> str:
    value = arguments.get(name, default)
    return value if isinstance(value, str) and value else default


def _json(value: JsonObject) -> str:
    return json.dumps(value, sort_keys=True)


def _object_schema(properties: JsonObject, required: list[str] | None = None) -> JsonObject:
    return {
        "type": "object",
        "properties": properties,
        "required": required or [],
        "additionalProperties": False,
    }


def _context_show_spec() -> ToolSpec:
    return ToolSpec(
        name="context.show",
        description="Show context budget and snapshot status for a session.",
        input_schema=_object_schema(
            {"session_id": {"type": "string", "minLength": 1}, "model": {"type": "string"}},
            ["session_id"],
        ),
        output_schema=_object_schema({"status": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _context_compact_spec() -> ToolSpec:
    return ToolSpec(
        name="context.compact",
        description="Create a durable context snapshot for a session.",
        input_schema=_object_schema(
            {"session_id": {"type": "string", "minLength": 1}, "model": {"type": "string"}},
            ["session_id"],
        ),
        output_schema=_object_schema({"snapshot": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="medium"),
    )


def _context_snapshots_spec() -> ToolSpec:
    return ToolSpec(
        name="context.snapshots",
        description="List durable context snapshots for a session.",
        input_schema=_object_schema(
            {"session_id": {"type": "string", "minLength": 1}},
            ["session_id"],
        ),
        output_schema=_object_schema({"snapshots": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _context_restore_spec() -> ToolSpec:
    return ToolSpec(
        name="context.restore",
        description="Select a durable context snapshot as active for future model requests.",
        input_schema=_object_schema(
            {"snapshot_id": {"type": "string", "minLength": 1}},
            ["snapshot_id"],
        ),
        output_schema=_object_schema(
            {"restored": {"type": "boolean"}, "snapshot": {"type": "object"}}
        ),
        permissions=ToolPermission(
            approval_required=True,
            mutation=True,
            working_root_required=False,
            risk="medium",
        ),
    )
