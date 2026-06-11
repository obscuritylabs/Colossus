"""Default policy implementation."""

from colossus.domain.policy import PolicyDecision
from colossus.domain.tools import ToolCall, ToolSpec


class DefaultPolicyEngine:
    """Simple capability policy that can be replaced by hardened adapters."""

    def decide_tool_call(self, spec: ToolSpec, call: ToolCall) -> PolicyDecision:
        if spec.name != call.name:
            return PolicyDecision(decision="deny", reason="Tool name mismatch.")
        if spec.permissions.network == "allow":
            return PolicyDecision(decision="requires_approval", reason="Tool may access network.")
        if spec.permissions.risk == "high":
            return PolicyDecision(decision="requires_approval", reason="Tool is high risk.")
        if spec.permissions.mutation:
            return PolicyDecision(decision="requires_approval", reason="Tool mutates state.")
        if spec.permissions.approval_required:
            return PolicyDecision(decision="requires_approval", reason="Tool requires approval.")
        return PolicyDecision(decision="allow", reason="Tool call matches declared policy.")
