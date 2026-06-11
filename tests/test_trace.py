from rich.console import Console

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
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


def test_trace_renderer_compact_collapses_payloads() -> None:
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
            output="first line\nsecond line",
        )
    )

    output = console.export_text()
    assert "tool call filesystem.read" in output
    assert "tool result filesystem.read" in output
    assert '"path": "pyproject.toml"' not in output
    assert "preview " not in output


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
