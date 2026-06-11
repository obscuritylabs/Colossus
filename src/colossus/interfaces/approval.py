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
    clear_approved_prompts: bool = True

    async def approve(self, call: ToolCall, decision: PolicyDecision) -> bool:
        clear_after_approval = self._can_clear_approved_prompt()
        if clear_after_approval:
            self._write_terminal_control(_SAVE_CURSOR)
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
        approved = Confirm.ask("Approve this tool call?", default=False, console=self.console)
        if approved and clear_after_approval:
            self._write_terminal_control(f"{_RESTORE_CURSOR}{_CLEAR_TO_END}")
        return approved

    def _can_clear_approved_prompt(self) -> bool:
        return (
            self.clear_approved_prompts
            and self.console.is_terminal
            and not self.console.is_dumb_terminal
        )

    def _write_terminal_control(self, sequence: str) -> None:
        self.console.file.write(sequence)
        self.console.file.flush()


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."


_SAVE_CURSOR = "\x1b7"
_RESTORE_CURSOR = "\x1b8"
_CLEAR_TO_END = "\x1b[J"
