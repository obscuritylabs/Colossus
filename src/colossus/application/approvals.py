"""Approval handlers used by application services."""

from colossus.domain.policy import PolicyDecision
from colossus.domain.tools import ToolCall


class DenyByDefaultApprovalHandler:
    """Non-interactive approval handler that denies approval-required actions."""

    async def approve(self, call: ToolCall, decision: PolicyDecision) -> bool:
        _ = call
        return decision.decision == "allow"


class AllowAllApprovalHandler:
    """Testing and explicitly trusted approval handler."""

    async def approve(self, call: ToolCall, decision: PolicyDecision) -> bool:
        _ = call, decision
        return True
