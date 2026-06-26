"""REPL transcript rendering for readable interactive sessions."""

import json
import re
import unicodedata
from dataclasses import dataclass, field

from rich import box
from rich.cells import cell_len, set_cell_size
from rich.console import Console
from rich.panel import Panel
from rich.status import Status
from rich.text import Text

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    ErrorEvent,
    FinalOutputEvent,
    HandoffEvent,
    ModelDeltaEvent,
    ReasoningSummaryEvent,
    ResearchStatusEvent,
    RiskAssessmentEvent,
    RunEvent,
    SubagentStatusEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.preferences import TranscriptStylePreference
from colossus.interfaces.trace import EventDisplayMode

_ANSI_ESCAPE_PATTERN = re.compile(
    r"\x1b(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))"
)
_PRESERVED_CONTROL_CHARS = {"\n", "\t"}


@dataclass
class TranscriptRenderTheme:
    user: str = "white on #30343a"
    assistant: str = "white"
    reasoning: str = "italic dim"
    tool: str = "bold cyan"
    tool_output: str = "green"
    approval: str = "bold yellow"
    risk: str = "bold magenta"
    research: str = "bold cyan"
    error: str = "bold red"
    meta: str = "dim"
    border: str = "dim"
    activity_spinner: str = "dots"


@dataclass
class ActivityIndicator:
    console: Console
    style: str = "dim"
    spinner: str = "dots"
    _status: Status | None = None
    _label: str | None = None

    @property
    def label(self) -> str | None:
        return self._label

    def start(self, label: str) -> None:
        if self._status is not None:
            self.update(label)
            return
        self._label = self._fit_label(label)
        self._status = self.console.status(
            self._label,
            spinner=self.spinner,
            spinner_style=self.style,
        )
        self._status.start()

    def update(self, label: str) -> None:
        if self._status is None:
            self.start(label)
            return
        self._label = self._fit_label(label)
        self._status.update(status=self._label)

    def stop(self) -> None:
        if self._status is not None:
            self._status.stop()
        self._status = None
        self._label = None

    def _fit_label(self, label: str) -> str:
        one_line = " ".join(label.splitlines())
        return _fit_cells(one_line, max(self.console.width - 10, 20))


@dataclass
class TranscriptRenderer:
    console: Console
    enabled: bool = True
    events_mode: EventDisplayMode = "compact"
    stream_model_output: bool = True
    show_reasoning: bool = True
    transcript_style: TranscriptStylePreference = "comfortable"
    theme: TranscriptRenderTheme = field(default_factory=TranscriptRenderTheme)
    argument_preview_chars: int = 240
    output_preview_chars: int = 360
    verbose_argument_preview_chars: int = 1000
    verbose_output_preview_chars: int = 1200
    activity_indicator: ActivityIndicator | None = None
    _rendered_model_output: bool = False
    _last_model_delta_ended_newline: bool = True
    _assistant_started: bool = False
    _activity_context: str = ""

    def __post_init__(self) -> None:
        if self.activity_indicator is None:
            self.activity_indicator = ActivityIndicator(
                self.console,
                style=self.theme.meta,
                spinner=self.theme.activity_spinner,
            )

    def sync_theme(self) -> None:
        if self.activity_indicator is not None:
            self.activity_indicator.style = self.theme.meta
            self.activity_indicator.spinner = self.theme.activity_spinner

    @property
    def rendered_model_output(self) -> bool:
        return self._rendered_model_output

    @property
    def activity_label(self) -> str | None:
        if self.activity_indicator is None:
            return None
        return self.activity_indicator.label

    @property
    def activity_spinner(self) -> str | None:
        if self.activity_indicator is None:
            return None
        return self.activity_indicator.spinner

    def begin_run(self, activity_context: str | None = None) -> None:
        self._stop_activity()
        self._rendered_model_output = False
        self._last_model_delta_ended_newline = True
        self._assistant_started = False
        self._activity_context = activity_context or ""
        self._set_activity("Thinking...")

    def end_run(self) -> None:
        self._stop_activity()
        if self._rendered_model_output and not self._last_model_delta_ended_newline:
            self.console.print()
        if self._rendered_model_output and self.transcript_style == "comfortable":
            self.console.print()
        self._last_model_delta_ended_newline = True
        self._assistant_started = False
        self._activity_context = ""

    def render_user_prompt(self, prompt: str) -> None:
        if self.transcript_style == "comfortable":
            self.console.print()
            self._panel("you", prompt, self.theme.user)
            return
        label = Text("you ", style=self.theme.meta)
        label.append(prompt, style=self.theme.user)
        self.console.print(label)

    def render_final_answer(self, text: str) -> None:
        text = _visible_transcript_text(text).strip()
        if not text:
            return
        self._stop_activity()
        if self.transcript_style == "comfortable":
            self.console.print()
            self.console.print(Text("agent", style=self.theme.meta))
            self.console.print(Text(text, style=self.theme.assistant))
            self.console.print()
        else:
            self.console.print(Text(text, style=self.theme.assistant))
        self._rendered_model_output = True
        self._last_model_delta_ended_newline = text.endswith("\n")

    def render_empty_response(self) -> None:
        self._stop_activity()
        self._render_status_block(
            "agent",
            "No assistant text returned.",
            self.theme.meta,
        )

    def render(self, event: RunEvent) -> None:
        if isinstance(event, ModelDeltaEvent):
            if self.stream_model_output:
                self._render_model_delta(event.text)
            return
        self._update_activity(event)
        if not self.enabled or self.events_mode == "off":
            return
        if isinstance(event, ReasoningSummaryEvent):
            if self.show_reasoning:
                self._render_reasoning(event)
            return
        if isinstance(event, FinalOutputEvent):
            if not self._rendered_model_output and self.stream_model_output:
                self.render_final_answer(event.text)
                return
            if self._rendered_model_output and not self._last_model_delta_ended_newline:
                self.console.print()
                self._last_model_delta_ended_newline = True
            if self.events_mode == "verbose":
                self._render_meta_block("done", "complete")
            return
        if isinstance(event, ToolCallRequestedEvent):
            self._render_tool_call(event)
            return
        if isinstance(event, ToolCallCompletedEvent):
            self._render_tool_result(event)
            return
        if isinstance(event, ApprovalRequestedEvent):
            if self.events_mode != "verbose":
                return
            self._render_status_block("approval requested", event.reason, self.theme.approval)
            return
        if isinstance(event, ApprovalAutoGrantedEvent):
            self._render_status_block("approval auto-granted", event.reason, self.theme.approval)
            return
        if isinstance(event, RiskAssessmentEvent):
            body = (
                f"{event.risk_level} decision={event.recommended_decision}\n"
                f"{event.summary}"
            )
            self._render_status_block("risk assessment", body, self.theme.risk)
            return
        if isinstance(event, SubagentStatusEvent):
            body = f"{event.status} {event.job_id}\n{event.task}"
            if event.message:
                body = f"{body}\n{event.message}"
            self._render_status_block("subagent", body, self.theme.tool)
            return
        if isinstance(event, ResearchStatusEvent):
            body = (
                f"{event.status} {event.research_id}\n"
                f"phase={event.phase} sources={event.sources_collected}"
            )
            if event.message:
                body = f"{body}\n{event.message}"
            self._render_status_block("research", body, self.theme.research)
            return
        if isinstance(event, ErrorEvent):
            self._render_status_block("error", event.message, self.theme.error)
            return
        if isinstance(event, HandoffEvent):
            reason = f"\n{event.reason}" if event.reason else ""
            self._render_status_block(
                "handoff",
                f"{event.from_agent} -> {event.to_agent}{reason}",
                self.theme.tool,
            )

    def _render_model_delta(self, text: str) -> None:
        if not text:
            return
        text = _visible_transcript_text(text)
        if not self._rendered_model_output:
            text = text.lstrip()
            if not _has_visible_cells(text):
                return
        self._stop_activity()
        if self.transcript_style == "comfortable" and not self._assistant_started:
            self.console.print()
            self.console.print(Text("agent", style=self.theme.meta))
            self._assistant_started = True
        self.console.print(
            text,
            end="",
            markup=False,
            highlight=False,
            style=self.theme.assistant,
        )
        self._rendered_model_output = True
        self._last_model_delta_ended_newline = text.endswith("\n")

    def _update_activity(self, event: RunEvent) -> None:
        if not self.enabled:
            return
        if isinstance(event, ReasoningSummaryEvent):
            self._set_activity("Thinking...")
            return
        if isinstance(event, ToolCallRequestedEvent):
            if event.name == "user.ask":
                self._stop_activity()
                return
            self._set_activity(f"Using {event.name}...")
            return
        if isinstance(event, ToolCallCompletedEvent):
            self._set_activity(f"Finished {event.name}; thinking...")
            return
        if isinstance(event, RiskAssessmentEvent):
            self._set_activity(f"Reviewing risk for {event.tool}...")
            return
        if isinstance(event, SubagentStatusEvent):
            self._set_activity(f"Subagent {event.status}: {event.job_id}")
            return
        if isinstance(event, ResearchStatusEvent):
            self._set_activity(f"Research {event.phase}...")
            return
        if isinstance(event, ApprovalRequestedEvent):
            self._stop_activity()
            return
        if isinstance(event, ApprovalAutoGrantedEvent):
            self._set_activity("Approval auto-granted; working...")
            return
        if isinstance(event, HandoffEvent):
            self._set_activity(f"Handing off to {event.to_agent}...")
            return
        if isinstance(event, FinalOutputEvent | ErrorEvent):
            self._stop_activity()

    def _set_activity(self, label: str) -> None:
        if not self.enabled or self.activity_indicator is None:
            return
        self.activity_indicator.update(self._activity_label(label))

    def _stop_activity(self) -> None:
        if self.activity_indicator is not None:
            self.activity_indicator.stop()

    def _activity_label(self, label: str) -> str:
        if not self._activity_context:
            return label
        return f"{label} | {self._activity_context}"

    def _render_reasoning(self, event: ReasoningSummaryEvent) -> None:
        summary = _truncate(event.summary, self.argument_preview_chars)
        self._render_status_block("thinking", summary, self.theme.reasoning)

    def _render_tool_call(self, event: ToolCallRequestedEvent) -> None:
        limit = (
            self.verbose_argument_preview_chars
            if self.events_mode == "verbose"
            else self.argument_preview_chars
        )
        arguments = _truncate(json.dumps(event.arguments, sort_keys=True), limit)
        body = f"{event.name}\nargs {arguments}\ncall_id={_short_id(event.call_id)}"
        self._render_status_block("tool call", body, self.theme.tool)

    def _render_tool_result(self, event: ToolCallCompletedEvent) -> None:
        limit = (
            self.verbose_output_preview_chars
            if self.events_mode == "verbose"
            else self.output_preview_chars
        )
        output = event.output.replace("\n", "\\n")
        preview = _truncate(output, limit)
        body = (
            f"{event.name} exit={event.exit_code} "
            f"bytes={len(event.output.encode('utf-8'))}"
        )
        if preview:
            body = f"{body}\n{preview}"
        self._render_status_block("tool result", body, self.theme.tool_output)

    def _render_meta_block(self, title: str, body: str) -> None:
        preview = _truncate(body, self.argument_preview_chars)
        self._render_status_block(title, preview, self.theme.meta)

    def _render_status_block(self, title: str, body: str, style: str) -> None:
        if self.transcript_style == "comfortable":
            self.console.print()
            self._panel(title, body, style)
            return
        line = Text(f"{title} ", style=style)
        if body:
            line.append(body.replace("\n", " "), style=self.theme.meta)
        self.console.print(line)

    def _panel(self, title: str, body: str, style: str) -> None:
        self.console.print(
            Panel(
                Text(body, style=style),
                title=f" {title} ",
                title_align="left",
                border_style=self.theme.border,
                box=box.SQUARE,
                padding=(1, 1),
                expand=True,
            )
        )


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."


def _fit_cells(value: str, width: int) -> str:
    if cell_len(value) <= width:
        return value
    return f"{set_cell_size(value, max(width - 1, 0))}…"


def _visible_transcript_text(value: str) -> str:
    without_ansi = _ANSI_ESCAPE_PATTERN.sub("", value)
    return "".join(
        character
        for character in without_ansi
        if character in _PRESERVED_CONTROL_CHARS
        or unicodedata.category(character) not in {"Cc", "Cf"}
    )


def _has_visible_cells(value: str) -> bool:
    return cell_len(value.strip()) > 0


def _short_id(value: str) -> str:
    return value[:8]
