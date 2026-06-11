"""Interactive approval prompts for terminal interfaces."""

import json
from dataclasses import dataclass

from rich.console import Console
from rich.prompt import Confirm

from colossus.domain.policy import PolicyDecision
from colossus.domain.tools import ToolCall


@dataclass
class RichApprovalHandler:
    console: Console
    argument_preview_chars: int = 1_200

    async def approve(self, call: ToolCall, decision: PolicyDecision) -> bool:
        self.console.print(
            f"[bold yellow]approval required[/bold yellow] {call.name} "
            f"[dim]call_id={call.call_id}[/dim]"
        )
        self.console.print(f"reason {decision.reason}", markup=False)
        arguments = json.dumps(call.arguments, sort_keys=True, indent=2)
        self.console.print(
            f"args {_truncate(arguments, self.argument_preview_chars)}",
            markup=False,
        )
        return Confirm.ask("Approve this tool call?", default=False, console=self.console)


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."
