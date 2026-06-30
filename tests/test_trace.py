import json

from rich.console import Console

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
    RiskAssessmentEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.interfaces.trace import RichRunEventRenderer


def test_trace_renderer_shows_tool_call_and_bounded_result() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(
        console,
        events_mode="verbose",
        output_preview_chars=24,
    )

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-1",
            name="filesystem.read",
            arguments={"path": "pyproject.toml", "max_lines": 30},
        )
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="filesystem.read",
            output="first line\nsecond line with more text than the preview",
        )
    )

    output = console.export_text()
    assert "tool call filesystem.read" in output
    assert '"path": "pyproject.toml"' in output
    assert "tool result filesystem.read" in output
    assert "bytes=" in output
    assert "preview " in output
    assert "first line\\nsecond li..." in output


def test_trace_renderer_compact_formats_filesystem_read_result() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, output_preview_chars=24)

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-1",
            name="filesystem.read",
            arguments={"path": "pyproject.toml", "max_lines": 30},
        )
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="filesystem.read",
            output=json.dumps(
                {
                    "path": "README.md",
                    "start_line": 1,
                    "line_count": 3,
                    "content": "# Title\n\n> quoted",
                    "truncated": False,
                }
            ),
        )
    )

    output = console.export_text()
    assert "tool call filesystem.read" not in output
    assert "read README.md" in output
    assert "lines=3" in output
    assert "1  # Title" in output
    assert "3  > quoted" in output
    assert '"path": "pyproject.toml"' not in output
    assert "preview " not in output


def test_trace_renderer_compact_formats_shell_result() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-1",
            name="shell.run",
            arguments={"argv": ["uv", "run", "pytest"], "cwd": "."},
        )
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="shell.run",
            output=json.dumps(
                {
                    "cwd": ".",
                    "exit_code": 0,
                    "stdout": "passed\n",
                    "stderr": "",
                }
            ),
        )
    )

    output = console.export_text()
    assert "tool call shell.run" not in output
    assert "ran uv run pytest" in output
    assert "exit=0 cwd=." in output
    assert "stdout" in output
    assert "passed" in output
    assert "\\n" not in output


def test_trace_renderer_compact_formats_git_status() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ToolCallRequestedEvent(call_id="call-1", name="git.status", arguments={})
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="git.status",
            output=json.dumps(
                {
                    "entries": [{"status": " M", "path": "src/app.py"}],
                    "raw": " M src/app.py\n",
                }
            ),
        )
    )

    output = console.export_text()
    assert "tool call git.status" not in output
    assert "git status 1 changed" in output
    assert "src/app.py" in output


def test_trace_renderer_compact_formats_git_diff() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ToolCallRequestedEvent(call_id="call-1", name="git.diff", arguments={})
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="git.diff",
            output=json.dumps(
                {
                    "diff": "--- a/file\n+++ b/file\n@@\n-old\n+new\n",
                    "stderr": "",
                    "exit_code": 0,
                }
            ),
        )
    )

    output = console.export_text()
    assert "tool call git.diff" not in output
    assert "git diff (+1 -1) exit=0" in output
    assert "+new" in output


def test_trace_renderer_formats_edit_results() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="patch.apply",
            output=json.dumps(
                {
                    "path": "src/app.py",
                    "replacements": 1,
                    "changed_line_ranges": [{"start": 10, "end": 12}],
                    "diff": "--- a/src/app.py\n+++ b/src/app.py\n@@\n-old\n+new\n",
                }
            ),
        )
    )

    output = console.export_text()
    assert "edited src/app.py" in output
    assert "(+1 -1)" in output
    assert "lines=10-12" in output
    assert "+new" in output


def test_trace_renderer_verbose_also_shows_edit_diff() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="verbose")

    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="patch.apply",
            output=json.dumps(
                {
                    "path": "src/app.py",
                    "replacements": 1,
                    "changed_line_ranges": [{"start": 10, "end": 10}],
                    "diff": "--- a/src/app.py\n+++ b/src/app.py\n@@\n-old\n+new\n",
                }
            ),
        )
    )

    output = console.export_text()
    assert "edited src/app.py" in output
    assert "(+1 -1)" in output
    assert "lines=10" in output
    assert "+new" in output


def test_trace_renderer_verbose_dumps_model_request() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="verbose")

    renderer.render(
        ModelRequestPreparedEvent(
            turn=0,
            model="demo-model",
            instructions="system prompt text",
            messages=({"role": "user", "content": "hello"},),
            tools=({"name": "memory.create", "description": "Save memory"},),
        )
    )

    output = console.export_text()
    assert "model request demo-model" in output
    assert '"instructions": "system prompt text"' in output
    assert '"content": "hello"' in output
    assert '"memory.create"' in output


def test_trace_renderer_compact_hides_model_request() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="compact")

    renderer.render(
        ModelRequestPreparedEvent(
            turn=0,
            model="demo-model",
            instructions="system prompt text",
            messages=({"role": "user", "content": "hello"},),
            tools=({"name": "memory.create", "description": "Save memory"},),
        )
    )

    assert console.export_text() == ""


def test_trace_renderer_can_be_disabled() -> None:
    console = Console(record=True)
    renderer = RichRunEventRenderer(console, enabled=False)

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-1",
            name="filesystem.read",
            arguments={},
        )
    )

    assert console.export_text() == ""


def test_trace_renderer_streams_model_delta_even_when_events_are_off() -> None:
    console = Console(record=True)
    renderer = RichRunEventRenderer(
        console,
        enabled=False,
        events_mode="off",
        stream_model_output=True,
    )

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="hel"))
    renderer.render(ModelDeltaEvent(text="lo"))
    renderer.end_run()

    assert renderer.rendered_model_output is True
    assert console.export_text() == "hello\n"


def test_trace_renderer_shows_reasoning_summary_not_raw_text() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ReasoningSummaryEvent(
            summary="Checked whether a tool is needed.",
            provider_format="openrouter",
            detail_id="reason-1",
        )
    )

    output = console.export_text()
    assert "thinking" in output
    assert "Checked whether a tool is needed." in output
    assert "reason-1" not in output


def test_trace_renderer_hides_done_after_streamed_output_in_compact_mode() -> None:
    console = Console(record=True)
    renderer = RichRunEventRenderer(console, stream_model_output=True)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="hello"))
    renderer.render(FinalOutputEvent(text="hello"))
    renderer.end_run()

    assert console.export_text() == "hello\n"


def test_trace_renderer_shows_done_after_streamed_output_in_verbose_mode() -> None:
    console = Console(record=True)
    renderer = RichRunEventRenderer(
        console,
        events_mode="verbose",
        stream_model_output=True,
    )

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="hello"))
    renderer.render(FinalOutputEvent(text="hello"))
    renderer.end_run()

    assert console.export_text() == "hello\ndone\n"


def test_trace_renderer_shows_risk_assessment() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        RiskAssessmentEvent(
            call_id="call-1",
            tool="shell.run",
            risk_level="high",
            summary="Deletes workspace files.",
            concerns=("destructive",),
            recommended_decision="deny",
            model_role="risk_evaluator",
            profile_name="risk",
        )
    )

    output = console.export_text()
    assert "risk assessment high" in output
    assert "decision=deny" in output
    assert "Deletes workspace files." in output


def test_trace_renderer_shows_auto_approval() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ApprovalAutoGrantedEvent(
            call_id="call-1",
            reason="Risk assessment auto-approved low-risk shell.run.",
        )
    )

    output = console.export_text()
    assert "approval auto-granted" in output
    assert "low-risk shell.run" in output
