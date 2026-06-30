"""REPL transcript rendering for readable interactive sessions."""

import json
import re
import unicodedata
from dataclasses import dataclass, field

from rich import box
from rich.cells import cell_len, set_cell_size
from rich.console import Console, RenderableType
from rich.markdown import Markdown
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
    ModelRequestPreparedEvent,
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


@dataclass(frozen=True)
class ShellResultSummary:
    command: str
    cwd: str
    exit_code: int
    stdout: str
    stderr: str


@dataclass(frozen=True)
class GitStatusSummary:
    entries: tuple[tuple[str, str], ...]


@dataclass(frozen=True)
class DiffResultSummary:
    title: str
    diff: str
    exit_code: int
    stderr: str
    additions: int
    deletions: int


_COMPACT_SEMANTIC_TOOL_CALLS = {
    "filesystem.read",
    "git.diff",
    "git.show",
    "git.status",
    "shell.run",
}


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
    render_streamed_markdown: bool = True
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
    _model_delta_buffer: list[str] = field(default_factory=list)
    _tool_call_arguments: dict[str, dict[str, object]] = field(default_factory=dict)

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
        self._model_delta_buffer.clear()
        self._tool_call_arguments.clear()
        self._set_activity("Thinking...")

    def end_run(self) -> None:
        self._render_buffered_model_output()
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
            self.console.print(Markdown(text))
            self.console.print()
        else:
            self.console.print(Markdown(text))
        self._rendered_model_output = True
        self._last_model_delta_ended_newline = text.endswith("\n")
        self._model_delta_buffer.clear()

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
                if self.render_streamed_markdown:
                    self._buffer_model_delta(event.text)
                else:
                    self._render_model_delta(event.text)
            return
        self._update_activity(event)
        if not self.enabled or self.events_mode == "off":
            return
        if isinstance(event, ReasoningSummaryEvent):
            if self.show_reasoning:
                self._render_reasoning(event)
            return
        if isinstance(event, ModelRequestPreparedEvent):
            if self.events_mode == "verbose":
                self._render_model_request(event)
            return
        if isinstance(event, FinalOutputEvent):
            rendered_final = False
            if not self._rendered_model_output:
                self._render_buffered_model_output(event.text)
                rendered_final = self._rendered_model_output
            if (
                self._rendered_model_output
                and not self._last_model_delta_ended_newline
                and not rendered_final
            ):
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

    def _buffer_model_delta(self, text: str) -> None:
        if not text:
            return
        text = _visible_transcript_text(text)
        if not self._model_delta_buffer:
            text = text.lstrip()
            if not _has_visible_cells(text):
                return
        self._model_delta_buffer.append(text)

    def _render_buffered_model_output(self, final_text: str = "") -> None:
        text = final_text or "".join(self._model_delta_buffer)
        if not text:
            return
        self.render_final_answer(text)

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

    def _render_model_request(self, event: ModelRequestPreparedEvent) -> None:
        body = _model_request_dump(event)
        self._render_status_block("model request", body, self.theme.meta)

    def _render_tool_call(self, event: ToolCallRequestedEvent) -> None:
        self._tool_call_arguments[event.call_id] = event.arguments
        if event.name in _COMPACT_SEMANTIC_TOOL_CALLS and self.events_mode != "verbose":
            return
        limit = (
            self.verbose_argument_preview_chars
            if self.events_mode == "verbose"
            else self.argument_preview_chars
        )
        arguments = _truncate(json.dumps(event.arguments, sort_keys=True), limit)
        body = f"{event.name}\nargs {arguments}\ncall_id={_short_id(event.call_id)}"
        self._render_status_block("tool call", body, self.theme.tool)

    def _render_tool_result(self, event: ToolCallCompletedEvent) -> None:
        edit_summary = _edit_result_summary(event.output)
        if edit_summary is not None:
            self._render_status_block(
                "edited",
                _edit_result_text(edit_summary),
                self.theme.tool_output,
            )
            return
        read_summary = _read_result_summary(event.name, event.output)
        if read_summary is not None:
            self._render_status_block(
                "read",
                _read_result_text(read_summary),
                self.theme.tool_output,
            )
            if self.events_mode == "verbose":
                raw_preview = _truncate(
                    event.output.replace("\n", "\\n"),
                    self.verbose_output_preview_chars,
                )
                self._render_status_block("raw result", raw_preview, self.theme.meta)
            return
        shell_summary = _shell_result_summary(
            event.name,
            event.output,
            self._tool_call_arguments.get(event.call_id, {}),
        )
        if shell_summary is not None:
            self._render_status_block(
                "shell",
                _shell_result_text(shell_summary),
                self.theme.tool_output,
            )
            if self.events_mode == "verbose":
                raw_preview = _truncate(
                    event.output.replace("\n", "\\n"),
                    self.verbose_output_preview_chars,
                )
                self._render_status_block("raw result", raw_preview, self.theme.meta)
            return
        git_status = _git_status_summary(event.name, event.output)
        if git_status is not None:
            self._render_status_block(
                "git status",
                _git_status_text(git_status),
                self.theme.tool_output,
            )
            return
        diff_summary = _diff_result_summary(event.name, event.output)
        if diff_summary is not None:
            self._render_status_block(
                diff_summary.title,
                _diff_result_text(diff_summary),
                self.theme.tool_output,
            )
            return
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

    def _render_status_block(self, title: str, body: str | Text, style: str) -> None:
        if self.transcript_style == "comfortable":
            self.console.print()
            self._panel(title, body, style)
            return
        line = Text(f"{title} ", style=style)
        if body:
            if isinstance(body, Text):
                line.append(body.plain.replace("\n", " "), style=self.theme.meta)
            else:
                line.append(body.replace("\n", " "), style=self.theme.meta)
        self.console.print(line)

    def _panel(self, title: str, body: str | Text, style: str) -> None:
        renderable: RenderableType = body if isinstance(body, Text) else Text(body, style=style)
        self.console.print(
            Panel(
                renderable,
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


def _edit_result_text(summary: EditResultSummary) -> Text:
    body = Text()
    body.append(summary.path)
    body.append(" ")
    body.append(f"(+{summary.additions}", style="green")
    body.append(" ")
    body.append(f"-{summary.deletions})", style="red")
    body.append(f"\nlines={summary.lines} replacements={summary.replacements}", style="dim")
    if summary.diff:
        body.append("\n")
        body.append_text(_styled_diff(summary.diff))
    return body


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


def _read_result_text(summary: ReadResultSummary) -> Text:
    body = Text()
    body.append(summary.path)
    body.append(
        f"  {summary.line_count} lines, {summary.content_bytes} bytes",
        style="dim",
    )
    if summary.truncated:
        body.append("  truncated", style="yellow")
    preview = _source_preview(summary.path, summary.content, summary.start_line)
    if preview:
        body.append("\n")
        body.append_text(preview)
    return body


def _shell_result_summary(
    name: str,
    output: str,
    arguments: dict[str, object],
) -> ShellResultSummary | None:
    if name != "shell.run":
        return None
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    exit_code = payload.get("exit_code")
    stdout = payload.get("stdout", "")
    stderr = payload.get("stderr", "")
    cwd = payload.get("cwd", ".")
    if not isinstance(exit_code, int):
        return None
    if not isinstance(stdout, str) or not isinstance(stderr, str) or not isinstance(cwd, str):
        return None
    argv = arguments.get("argv")
    command = "shell.run"
    if isinstance(argv, list):
        command = " ".join(str(part) for part in argv)
    return ShellResultSummary(
        command=command,
        cwd=cwd,
        exit_code=exit_code,
        stdout=stdout,
        stderr=stderr,
    )


def _shell_result_text(summary: ShellResultSummary) -> Text:
    body = Text()
    body.append("$ ", style="dim")
    body.append(summary.command)
    exit_style = "green" if summary.exit_code == 0 else "red"
    body.append(f"\nexit={summary.exit_code}", style=exit_style)
    body.append(f" cwd={summary.cwd}", style="dim")
    output = _shell_output_preview(summary)
    if output:
        body.append("\n")
        body.append_text(output)
    return body


def _shell_output_preview(summary: ShellResultSummary) -> Text:
    rendered = Text()
    if summary.stdout:
        rendered.append("stdout\n", style="dim")
        rendered.append_text(_source_preview("stdout", summary.stdout, 1, limit=10))
    if summary.stderr:
        rendered.append("stderr\n", style="red")
        rendered.append_text(_source_preview("stderr", summary.stderr, 1, limit=10))
    return rendered


def _git_status_summary(name: str, output: str) -> GitStatusSummary | None:
    if name != "git.status":
        return None
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    entries = payload.get("entries")
    if not isinstance(entries, list):
        return None
    parsed: list[tuple[str, str]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        status = entry.get("status")
        path = entry.get("path")
        if isinstance(status, str) and isinstance(path, str):
            parsed.append((status, path))
    return GitStatusSummary(entries=tuple(parsed))


def _git_status_text(summary: GitStatusSummary) -> Text:
    body = Text()
    if not summary.entries:
        body.append("clean", style="green")
        return body
    body.append(f"{len(summary.entries)} changed", style="yellow")
    for status, path in summary.entries[:20]:
        body.append("\n")
        body.append(status, style="yellow")
        body.append("  ")
        body.append(path)
    if len(summary.entries) > 20:
        body.append(f"\n... {len(summary.entries) - 20} more", style="dim")
    return body


def _diff_result_summary(name: str, output: str) -> DiffResultSummary | None:
    if name not in {"git.diff", "git.show"}:
        return None
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    diff_key = "diff" if name == "git.diff" else "output"
    diff = payload.get(diff_key)
    stderr = payload.get("stderr", "")
    exit_code = payload.get("exit_code", 0)
    if not isinstance(diff, str) or not isinstance(stderr, str) or not isinstance(exit_code, int):
        return None
    additions, deletions = _diff_counts(diff)
    return DiffResultSummary(
        title="git diff" if name == "git.diff" else "git show",
        diff=diff,
        exit_code=exit_code,
        stderr=stderr,
        additions=additions,
        deletions=deletions,
    )


def _diff_result_text(summary: DiffResultSummary) -> Text:
    body = Text()
    body.append(f"(+{summary.additions}", style="green")
    body.append(" ")
    body.append(f"-{summary.deletions})", style="red")
    body.append(f" exit={summary.exit_code}", style="dim")
    if summary.diff:
        body.append("\n")
        body.append_text(_styled_diff(summary.diff))
    if summary.stderr:
        body.append("\nstderr\n", style="red")
        body.append_text(_source_preview("stderr", summary.stderr, 1, limit=8))
    return body


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
