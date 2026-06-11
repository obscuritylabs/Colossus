from rich.console import Console

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    ErrorEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ReasoningSummaryEvent,
    RiskAssessmentEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.interfaces.transcript import TranscriptRenderer, TranscriptRenderTheme


def test_transcript_renderer_renders_user_block_with_spacing() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render_user_prompt("read the codebase")

    output = console.export_text()
    assert "you" in output
    assert "read the codebase" in output


def test_transcript_renderer_streams_assistant_without_duplicate_final_output() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="hel"))
    renderer.render(ModelDeltaEvent(text="lo"))
    renderer.render(FinalOutputEvent(text="hello"))
    renderer.end_run()

    output = console.export_text()
    assert "agent" in output
    assert output.count("hello") == 1
    assert "done" not in output


def test_transcript_renderer_ignores_leading_whitespace_delta_before_answer() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="\n\n   "))
    assert not renderer.rendered_model_output
    renderer.render(FinalOutputEvent(text="real answer"))
    renderer.end_run()

    output = console.export_text()
    assert output.count("agent") == 1
    assert output.count("real answer") == 1


def test_transcript_renderer_ignores_invisible_delta_before_answer() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="\u200b\u200d\ufeff"))
    renderer.render(ModelDeltaEvent(text="\x1b[32m\x1b[0m"))
    assert not renderer.rendered_model_output
    renderer.render(ModelDeltaEvent(text="\x1b[32mvisible\x1b[0m"))
    renderer.end_run()

    output = console.export_text()
    assert output.count("agent") == 1
    assert "visible" in output
    assert "\x1b" not in output


def test_transcript_renderer_ignores_whitespace_only_final_answer() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render_final_answer("\n\n   ")

    output = console.export_text()
    assert "agent" not in output


def test_transcript_renderer_can_render_empty_response_notice() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render_empty_response()

    output = console.export_text()
    assert "agent" in output
    assert "No assistant text returned." in output


def test_transcript_renderer_reasoning_summary_hides_detail_id() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ReasoningSummaryEvent(
            summary="I should inspect the project files.",
            provider_format="openrouter",
            detail_id="hidden-detail",
        )
    )

    output = console.export_text()
    assert "thinking" in output
    assert "inspect the project files" in output
    assert "hidden-detail" not in output


def test_transcript_renderer_tool_preview_is_collapsed_in_compact_mode() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, output_preview_chars=24)

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-123456",
            name="filesystem.read",
            arguments={"path": "pyproject.toml", "max_lines": 20},
        )
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-123456",
            name="filesystem.read",
            output="first line\nsecond line with more text",
        )
    )

    output = console.export_text()
    assert "tool call" in output
    assert "filesystem.read" in output
    assert "call-123" in output
    assert "tool result" in output
    assert "first line\\nsecond li..." in output


def test_transcript_renderer_verbose_shows_larger_tool_details_and_done() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(
        console,
        events_mode="verbose",
        stream_model_output=True,
        output_preview_chars=8,
        verbose_output_preview_chars=64,
    )

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="hello"))
    renderer.render(FinalOutputEvent(text="hello"))
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="shell.run",
            output="first line\nsecond line",
        )
    )
    renderer.end_run()

    output = console.export_text()
    assert "done" in output
    assert output.count("hello") == 1
    assert "first line\\nsecond line" in output


def test_transcript_renderer_status_blocks_are_distinct() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(ApprovalRequestedEvent(call_id="call-1", reason="Needs permission."))
    renderer.render(
        ApprovalAutoGrantedEvent(call_id="call-2", reason="Low-risk command.")
    )
    renderer.render(
        RiskAssessmentEvent(
            call_id="call-3",
            tool="shell.run",
            risk_level="high",
            summary="Deletes files.",
            recommended_decision="deny",
            model_role="risk_evaluator",
            profile_name="risk",
        )
    )
    renderer.render(ErrorEvent(message="Something failed."))

    output = console.export_text()
    assert "approval requested" in output
    assert "approval auto-granted" in output
    assert "risk assessment" in output
    assert "Deletes files." in output
    assert "error" in output
    assert "Something failed." in output


def test_transcript_renderer_events_off_still_streams_assistant() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, enabled=False, events_mode="off")

    renderer.begin_run()
    renderer.render(ReasoningSummaryEvent(summary="hidden"))
    renderer.render(ModelDeltaEvent(text="hello"))
    renderer.end_run()

    output = console.export_text()
    assert "hello" in output
    assert "hidden" not in output


def test_transcript_renderer_events_off_tracks_activity_without_event_blocks() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, events_mode="off")

    renderer.begin_run()
    assert renderer.activity_label == "Thinking..."
    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-123456",
            name="filesystem.list",
            arguments={"path": ".", "max_entries": 20},
        )
    )
    assert renderer.activity_label == "Using filesystem.list..."
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-123456",
            name="filesystem.list",
            output='{"entries": []}',
        )
    )
    assert renderer.activity_label == "Finished filesystem.list; thinking..."
    renderer.render(
        RiskAssessmentEvent(
            call_id="call-2",
            tool="shell.run",
            risk_level="low",
            summary="Echo command.",
            recommended_decision="allow",
            model_role="risk_evaluator",
            profile_name="primary",
        )
    )
    assert renderer.activity_label == "Reviewing risk for shell.run..."
    renderer.end_run()

    output = console.export_text()
    assert "tool call" not in output
    assert "filesystem.list" not in output
    assert "risk assessment" not in output
    assert renderer.activity_label is None


def test_transcript_renderer_streaming_delta_stops_activity_before_output() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, events_mode="off")

    renderer.begin_run(activity_context="mode=single model=primary:demo")
    assert renderer.activity_label == "Thinking... | mode=single model=primary:demo"
    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-123456",
            name="filesystem.read",
            arguments={"path": "README.md"},
        )
    )
    assert renderer.activity_label == "Using filesystem.read... | mode=single model=primary:demo"
    renderer.render(ModelDeltaEvent(text=""))
    assert renderer.activity_label == "Using filesystem.read... | mode=single model=primary:demo"
    renderer.render(ModelDeltaEvent(text="hello"))
    assert renderer.activity_label is None
    renderer.end_run()

    output = console.export_text()
    assert "hello" in output
    assert "filesystem.read" not in output
    assert renderer.activity_label is None


def test_transcript_renderer_uses_theme_activity_spinner() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(
        console,
        events_mode="off",
        theme=TranscriptRenderTheme(activity_spinner="line"),
    )

    assert renderer.activity_spinner == "line"
    renderer.theme = TranscriptRenderTheme(activity_spinner="arc")
    renderer.sync_theme()

    assert renderer.activity_spinner == "arc"
