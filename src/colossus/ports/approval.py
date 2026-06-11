"""Approval port."""

from typing import Protocol

from colossus.domain.policy import PolicyDecision
from colossus.domain.tools import ToolCall


class ApprovalHandler(Protocol):
    async def approve(self, call: ToolCall, decision: PolicyDecision) -> bool:
        """Return whether the requested action is approved."""
        ...
