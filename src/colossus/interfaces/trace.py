"""Terminal rendering for observable agent run events."""

import json
from dataclasses import dataclass, field
from typing import Literal

from rich.console import Console
from rich.text import Text

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
    ResearchStatusEvent,
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
    research: str = "bold cyan"


@dataclass(frozen=True)
class EditResultSummary:
    path: str
    diff: str
    lines: str
    replacements: str
    additions: int
    deletions: int


@dataclass(frozen=True)
class ReadResultSummary:
    path: str
    content: str
    start_line: int
    line_count: int
    truncated: bool
    content_bytes: int


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
        if isinstance(event, ModelRequestPreparedEvent):
            if self.events_mode != "verbose":
                return
            self.console.print(
                f"{_label('model request', self.theme.thinking)} "
                f"{event.model} [dim]turn={event.turn} messages={len(event.messages)} "
                f"tools={len(event.tools)}[/dim]"
            )
            self.console.print(_model_request_dump(event), markup=False)
            return
        if isinstance(event, FinalOutputEvent):
            if self._rendered_model_output and not self._last_model_delta_ended_newline:
                self.console.print()
                self._last_model_delta_ended_newline = True
            if self.events_mode == "verbose":
                self.console.print(_label("done", self.theme.done))
            return
        if isinstance(event, ToolCallRequestedEvent):
            if event.name == "filesystem.read" and self.events_mode != "verbose":
                return
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
        if isinstance(event, ResearchStatusEvent):
            message = _truncate(event.message, self.argument_preview_chars)
            self.console.print(
                f"{_label('research', self.theme.research)} {event.phase} "
                f"[dim]run={event.research_id} status={event.status} "
                f"sources={event.sources_collected}[/dim]"
            )
            if message and self.events_mode == "verbose":
                self.console.print(message, markup=False)
            return
        if isinstance(event, ToolCallCompletedEvent):
            edit_summary = _edit_result_summary(event.output)
            if edit_summary is not None:
                header = Text("edited ", style=self.theme.tool_result)
                header.append(edit_summary.path)
                header.append(" ")
                header.append(f"(+{edit_summary.additions}", style="green")
                header.append(" ")
                header.append(f"-{edit_summary.deletions})", style="red")
                header.append(
                    f" lines={edit_summary.lines} "
                    f"replacements={edit_summary.replacements} "
                    f"bytes={len(event.output.encode('utf-8'))}",
                    style="dim",
                )
                self.console.print(header)
                if edit_summary.diff:
                    self.console.print(_styled_diff(edit_summary.diff))
                return
            read_summary = _read_result_summary(event.name, event.output)
            if read_summary is not None:
                header = Text("read ", style=self.theme.tool_result)
                header.append(read_summary.path)
                header.append(
                    f" lines={read_summary.line_count} "
                    f"bytes={read_summary.content_bytes}",
                    style="dim",
                )
                if read_summary.truncated:
                    header.append(" truncated", style="yellow")
                self.console.print(header)
                preview = _source_preview(
                    read_summary.path,
                    read_summary.content,
                    read_summary.start_line,
                )
                if preview:
                    self.console.print(preview)
                if self.events_mode == "verbose":
                    raw_preview = _truncate(
                        event.output.replace("\n", "\\n"),
                        self.output_preview_chars,
                    )
                    self.console.print(f"raw {raw_preview}", markup=False)
                return
            output_preview = _truncate(
                event.output.replace("\n", "\\n"),
                self.output_preview_chars,
            )
            self.console.print(
                f"{_label('tool result', self.theme.tool_result)} {event.name} "
                f"[dim]call_id={event.call_id} exit={event.exit_code} "
                f"bytes={len(event.output.encode('utf-8'))}[/dim]"
            )
            if output_preview and self.events_mode == "verbose":
                self.console.print(f"preview {output_preview}", markup=False)


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."


def _model_request_dump(event: ModelRequestPreparedEvent) -> str:
    return json.dumps(
        {
            "instructions": event.instructions,
            "messages": event.messages,
            "model": event.model,
            "tools": list(event.tools),
            "turn": event.turn,
        },
        indent=2,
        sort_keys=True,
    )


def _edit_result_summary(output: str) -> EditResultSummary | None:
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    path = payload.get("path")
    diff = payload.get("diff")
    if not isinstance(path, str) or not isinstance(diff, str):
        return None
    replacements = payload.get("replacements", "")
    if "bytes_written" in payload:
        replacements = "write"
    additions, deletions = _diff_counts(diff)
    return EditResultSummary(
        path=path,
        diff=diff,
        lines=_line_ranges(payload.get("changed_line_ranges")),
        replacements=str(replacements),
        additions=additions,
        deletions=deletions,
    )


def _read_result_summary(name: str, output: str) -> ReadResultSummary | None:
    if name != "filesystem.read":
        return None
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    path = payload.get("path")
    content = payload.get("content")
    start_line = payload.get("start_line", 1)
    line_count = payload.get("line_count")
    truncated = payload.get("truncated", False)
    if not isinstance(path, str) or not isinstance(content, str):
        return None
    if not isinstance(start_line, int):
        start_line = 1
    if not isinstance(line_count, int):
        line_count = len(content.splitlines())
    if not isinstance(truncated, bool):
        truncated = False
    return ReadResultSummary(
        path=path,
        content=content,
        start_line=start_line,
        line_count=line_count,
        truncated=truncated,
        content_bytes=len(content.encode("utf-8")),
    )


def _source_preview(path: str, content: str, start_line: int, limit: int = 16) -> Text:
    lines = content.splitlines()
    rendered = Text()
    for line_number, line in _preview_lines(lines, start_line, limit):
        if line_number is None:
            rendered.append("  ...\n", style="dim")
            continue
        rendered.append(f"{line_number:>4}  ", style="dim")
        rendered.append(line, style=_source_line_style(path, line))
        rendered.append("\n")
    return rendered


def _preview_lines(
    lines: list[str],
    start_line: int,
    limit: int,
) -> list[tuple[int | None, str]]:
    numbered: list[tuple[int | None, str]] = [
        (start_line + index, line) for index, line in enumerate(lines)
    ]
    if len(numbered) <= limit:
        return numbered
    head_count = max(1, limit // 2)
    tail_count = max(1, limit - head_count - 1)
    return [*numbered[:head_count], (None, ""), *numbered[-tail_count:]]


def _source_line_style(path: str, line: str) -> str:
    if path.endswith((".md", ".markdown")):
        stripped = line.lstrip()
        if stripped.startswith("#"):
            return "bold cyan"
        if stripped.startswith(">"):
            return "italic dim"
        if stripped.startswith("```"):
            return "magenta"
        if set(stripped) <= {"|", "-", ":", " "} and "|" in stripped:
            return "dim"
        if stripped.startswith(("-", "*", "+")):
            return "cyan"
    return ""


def _diff_counts(diff: str) -> tuple[int, int]:
    additions = 0
    deletions = 0
    for line in diff.splitlines():
        if line.startswith("+++") or line.startswith("---"):
            continue
        if line.startswith("+"):
            additions += 1
        elif line.startswith("-"):
            deletions += 1
    return additions, deletions


def _styled_diff(diff: str) -> Text:
    rendered = Text()
    for line in diff.splitlines():
        style = ""
        if line.startswith("@@"):
            style = "cyan"
        elif line.startswith("+++") or line.startswith("---"):
            style = "dim"
        elif line.startswith("+"):
            style = "green"
        elif line.startswith("-"):
            style = "red"
        rendered.append(line, style=style)
        rendered.append("\n")
    return rendered


def _line_ranges(value: object) -> str:
    if not isinstance(value, list) or not value:
        return "unknown"
    ranges: list[str] = []
    for item in value:
        if not isinstance(item, dict):
            continue
        start = item.get("start")
        end = item.get("end")
        if not isinstance(start, int) or not isinstance(end, int):
            continue
        ranges.append(str(start) if start == end else f"{start}-{end}")
    return ",".join(ranges) or "unknown"


def _label(value: str, style: str) -> str:
    return f"[{style}]{value}[/{style}]"
