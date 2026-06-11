"""Policy engine port."""

from typing import Protocol

from colossus.domain.policy import PolicyDecision
from colossus.domain.tools import ToolCall, ToolSpec


class PolicyEngine(Protocol):
    def decide_tool_call(self, spec: ToolSpec, call: ToolCall) -> PolicyDecision:
        """Evaluate a tool call against policy."""
        ...
