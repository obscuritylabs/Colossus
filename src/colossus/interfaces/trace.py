"""Terminal rendering for observable agent run events."""

import json
from dataclasses import dataclass, field
from typing import Literal

from rich.console import Console

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ReasoningSummaryEvent,
    RiskAssessmentEvent,
    RunEvent,
    SubagentStatusEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)

EventDisplayMode = Literal["compact", "verbose", "off"]


@dataclass
class TraceRenderTheme:
    thinking: str = "bold cyan"
    done: str = "bold green"
    tool_call: str = "bold blue"
    tool_result: str = "bold green"
    approval_requested: str = "bold yellow"
    approval_auto_granted: str = "bold green"
    risk_assessment: str = "bold magenta"


@dataclass
class RichRunEventRenderer:
    console: Console
    enabled: bool = True
    events_mode: EventDisplayMode = "compact"
    stream_model_output: bool = False
    show_reasoning: bool = True
    theme: TraceRenderTheme = field(default_factory=TraceRenderTheme)
    argument_preview_chars: int = 500
    output_preview_chars: int = 320
    _rendered_model_output: bool = False
    _last_model_delta_ended_newline: bool = True

    @property
    def rendered_model_output(self) -> bool:
        return self._rendered_model_output

    def begin_run(self) -> None:
        self._rendered_model_output = False
        self._last_model_delta_ended_newline = True

    def end_run(self) -> None:
        if self._rendered_model_output and not self._last_model_delta_ended_newline:
            self.console.print()
        self._last_model_delta_ended_newline = True

    def render(self, event: RunEvent) -> None:
        if isinstance(event, ModelDeltaEvent):
            if self.stream_model_output:
                self.console.print(event.text, end="", markup=False, highlight=False)
                self._rendered_model_output = True
                self._last_model_delta_ended_newline = event.text.endswith("\n")
            return
        if not self.enabled or self.events_mode == "off":
            return
        if isinstance(event, ReasoningSummaryEvent):
            if not self.show_reasoning:
                return
            summary = _truncate(event.summary, self.argument_preview_chars)
            self.console.print(f"{_label('thinking', self.theme.thinking)} ", end="")
            self.console.print(summary, markup=False)
            return
        if isinstance(event, FinalOutputEvent):
            if self._rendered_model_output and not self._last_model_delta_ended_newline:
                self.console.print()
                self._last_model_delta_ended_newline = True
            if self.events_mode == "verbose":
                self.console.print(_label("done", self.theme.done))
            return
        if isinstance(event, ToolCallRequestedEvent):
            arguments = _truncate(
                json.dumps(event.arguments, sort_keys=True),
                self.argument_preview_chars,
            )
            self.console.print(
                f"{_label('tool call', self.theme.tool_call)} {event.name} "
                f"[dim]call_id={event.call_id}[/dim]"
            )
            if self.events_mode == "verbose":
                self.console.print(f"args {arguments}", markup=False)
            return
        if isinstance(event, ApprovalRequestedEvent):
            reason = _truncate(event.reason, self.argument_preview_chars)
            self.console.print(
                f"{_label('approval requested', self.theme.approval_requested)} "
                f"[dim]call_id={event.call_id}[/dim] {reason}"
            )
            return
        if isinstance(event, ApprovalAutoGrantedEvent):
            reason = _truncate(event.reason, self.argument_preview_chars)
            self.console.print(
                f"{_label('approval auto-granted', self.theme.approval_auto_granted)} "
                f"[dim]call_id={event.call_id}[/dim] {reason}"
            )
            return
        if isinstance(event, RiskAssessmentEvent):
            summary = _truncate(event.summary, self.argument_preview_chars)
            self.console.print(
                f"{_label('risk assessment', self.theme.risk_assessment)} {event.risk_level} "
                f"[dim]tool={event.tool} decision={event.recommended_decision} "
                f"role={event.model_role} profile={event.profile_name}[/dim]"
            )
            self.console.print(f"summary {summary}", markup=False)
            return
        if isinstance(event, SubagentStatusEvent):
            task = _truncate(event.task, self.argument_preview_chars)
            self.console.print(
                f"{_label('subagent', self.theme.tool_call)} {event.status} "
                f"[dim]job={event.job_id} role={event.role}[/dim]"
            )
            if self.events_mode == "verbose":
                self.console.print(f"task {task}", markup=False)
            return
        if isinstance(event, ToolCallCompletedEvent):
            preview = _truncate(event.output.replace("\n", "\\n"), self.output_preview_chars)
            self.console.print(
                f"{_label('tool result', self.theme.tool_result)} {event.name} "
                f"[dim]call_id={event.call_id} exit={event.exit_code} "
                f"bytes={len(event.output.encode('utf-8'))}[/dim]"
            )
            if preview and self.events_mode == "verbose":
                self.console.print(f"preview {preview}", markup=False)


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."


def _label(value: str, style: str) -> str:
    return f"[{style}]{value}[/{style}]"
