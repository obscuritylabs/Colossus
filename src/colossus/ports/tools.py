"""Tool ports."""

from typing import Protocol

from colossus.domain.tools import ToolCall, ToolResult, ToolSpec


class ToolRegistry(Protocol):
    def list_specs(self) -> tuple[ToolSpec, ...]:
        """Return available tool specifications."""
        ...

    def get_spec(self, name: str) -> ToolSpec | None:
        """Return a tool spec by name."""
        ...


class ToolExecutor(Protocol):
    async def execute(self, call: ToolCall) -> ToolResult:
        """Execute a validated tool call."""
        ...
