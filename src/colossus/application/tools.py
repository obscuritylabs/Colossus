"""Tool registry and validation service."""

import json
from collections.abc import Awaitable, Callable

from jsonschema import ValidationError, validate

from colossus.domain.errors import ToolExecutionError
from colossus.domain.tools import ToolCall, ToolResult, ToolSpec

ToolHandler = Callable[[dict[str, object]], Awaitable[str]]


class InMemoryToolRegistry:
    def __init__(self, specs: tuple[ToolSpec, ...]) -> None:
        names = [spec.name for spec in specs]
        duplicates = sorted({name for name in names if names.count(name) > 1})
        if duplicates:
            raise ValueError(f"Duplicate tool names are not allowed: {', '.join(duplicates)}")
        self._specs = {spec.name: spec for spec in specs}

    def list_specs(self) -> tuple[ToolSpec, ...]:
        return tuple(self._specs.values())

    def get_spec(self, name: str) -> ToolSpec | None:
        return self._specs.get(name)


class FunctionToolExecutor:
    def __init__(self, handlers: dict[str, ToolHandler], registry: InMemoryToolRegistry) -> None:
        self._handlers = handlers
        self._registry = registry

    async def execute(self, call: ToolCall) -> ToolResult:
        spec = self._registry.get_spec(call.name)
        if spec is None:
            raise ToolExecutionError(f"Unknown tool: {call.name}")
        validate_tool_call(spec, call)
        handler = self._handlers.get(call.name)
        if handler is None:
            raise ToolExecutionError(f"No handler registered for tool: {call.name}")
        output = await handler(call.arguments)
        if spec.output_schema is not None:
            try:
                validate(instance=json.loads(output), schema=spec.output_schema)
            except json.JSONDecodeError as exc:
                raise ToolExecutionError(f"Tool {call.name} returned non-JSON output.") from exc
            except ValidationError as exc:
                raise ToolExecutionError(
                    f"Invalid output for {call.name}: {exc.message}"
                ) from exc
        if len(output.encode("utf-8")) > spec.max_output_bytes:
            output = output.encode("utf-8")[: spec.max_output_bytes].decode(
                "utf-8",
                errors="replace",
            )
        return ToolResult(call_id=call.call_id, name=call.name, output=output)


def validate_tool_call(spec: ToolSpec, call: ToolCall) -> None:
    if spec.name != call.name:
        raise ToolExecutionError(f"Tool call/spec mismatch: {call.name} != {spec.name}")
    try:
        validate(instance=call.arguments, schema=spec.input_schema)
    except ValidationError as exc:
        raise ToolExecutionError(f"Invalid arguments for {call.name}: {exc.message}") from exc
