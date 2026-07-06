import json

from rich.console import Console

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    ContextPreparedEvent,
    ErrorEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
    ResearchProgressEvent,
    ResearchStatusEvent,
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


def test_transcript_renderer_buffers_assistant_and_renders_final_markdown() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="# Research"))
    renderer.render(ModelDeltaEvent(text=" Report"))
    assert not renderer.rendered_model_output
    assert "Research Report" not in console.export_text()

    renderer.render(FinalOutputEvent(text="# Research Report\n\n- Finding one"))
    renderer.end_run()

    output = console.export_text()
    assert "agent" in output
    assert "Research Report" in output
    assert "Finding one" in output
    assert "# Research Report" not in output
    assert "done" not in output


def test_transcript_renderer_raw_streams_assistant_without_duplicate_final_output() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, render_streamed_markdown=False)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="hel"))
    renderer.render(ModelDeltaEvent(text="lo"))
    renderer.render(FinalOutputEvent(text="hello"))
    renderer.end_run()

    output = console.export_text()
    assert "agent" in output
    assert output.count("hello") == 1
    assert "done" not in output


def test_transcript_renderer_abort_discards_buffered_markdown_output() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.begin_run()
    renderer.render(ModelDeltaEvent(text="I'll do zyx. "))
    renderer.render(ModelDeltaEvent(text="Now I'll do xyz."))
    renderer.abort_run()

    output = console.export_text()
    assert "I'll do zyx" not in output
    assert "Now I'll do xyz" not in output
    assert renderer.rendered_model_output is False


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


def test_transcript_renderer_renders_final_answer_markdown() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render_final_answer("# Research Report\n\n- Finding one")

    output = console.export_text()
    assert "Research Report" in output
    assert "Finding one" in output
    assert "# Research Report" not in output


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


def test_transcript_renderer_comfortable_formats_research_progress_outline() -> None:
    console = Console(record=True, width=160)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ResearchProgressEvent(
            research_id="research-1",
            phase="collecting",
            action="web",
            status="completed",
            message="Web search returned 4 result(s).",
            query="deep research progress telemetry",
            source_kind="web",
            current=1,
            total=2,
            sources_collected=4,
            details={"results": 4, "added": 4, "configured": True, "approved": True},
        )
    )

    output = console.export_text()
    assert "research 2 - collecting" in output
    assert "* web completed 1/2" in output
    assert "results=4" in output
    assert "sources=4" in output
    assert 'query="deep research progress telemetry"' in output
    assert "┌─  research progress" not in output
    assert renderer.activity_label is not None
    assert "Finished research collecting/web 1/2..." in renderer.activity_label


def test_transcript_renderer_dense_formats_research_progress_one_line() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, transcript_style="dense")

    renderer.render(
        ResearchProgressEvent(
            research_id="research-1",
            phase="workers",
            action="claim",
            status="completed",
            message="Extracted claim from [R1].",
            current=1,
            total=3,
            sources_collected=3,
            claims_collected=1,
            details={"label": "R1", "title": "docs/example.md", "kind": "repo"},
        )
    )

    output = console.export_text()
    assert "research 3 - workers" in output
    assert "* claim completed 1/3" in output
    assert "claims=1" in output
    assert "[R1]" in output
    assert "docs/example.md" in output
    assert "Extracted claim from [R1]." not in output


def test_transcript_renderer_compact_skips_started_claim_progress() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ResearchProgressEvent(
            research_id="research-1",
            phase="workers",
            action="claim",
            status="started",
            message="Extracting claim from [R2] Noisy source title.",
            current=2,
            total=18,
            sources_collected=18,
            claims_collected=1,
            details={"label": "R2", "title": "Noisy source title", "kind": "web"},
        )
    )

    assert console.export_text() == ""
    assert renderer.activity_label is not None
    assert "Research workers/claim 2/18..." in renderer.activity_label


def test_transcript_renderer_research_status_regression() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ResearchStatusEvent(
            research_id="research-1",
            status="running",
            phase="synthesis",
            message="Synthesizing cited research report.",
            sources_collected=17,
        )
    )

    output = console.export_text()
    assert "research" in output
    assert "running research-1" in output
    assert "phase=synthesis sources=17" in output
    assert "Synthesizing cited research report." in output


def test_transcript_renderer_formats_filesystem_read_in_compact_mode() -> None:
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
            output=json.dumps(
                {
                    "path": "notes.md",
                    "start_line": 10,
                    "line_count": 4,
                    "content": "# Notes\n\n- One\n> Quote",
                    "truncated": False,
                }
            ),
        )
    )

    output = console.export_text()
    assert "tool call" not in output
    assert "read" in output
    assert "notes.md" in output
    assert "4 lines" in output
    assert "10  # Notes" in output
    assert "13  > Quote" in output
    assert "\\n" not in output


def test_transcript_renderer_formats_shell_result_in_compact_mode() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-123456",
            name="shell.run",
            arguments={"argv": ["uv", "run", "pytest"], "cwd": "."},
        )
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-123456",
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
    assert "tool call" not in output
    assert "shell" in output
    assert "$ uv run pytest" in output
    assert "exit=0 cwd=." in output
    assert "stdout" in output
    assert "passed" in output
    assert "\\n" not in output


def test_transcript_renderer_formats_git_status_in_compact_mode() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ToolCallRequestedEvent(call_id="call-123456", name="git.status", arguments={})
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-123456",
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
    assert "tool call" not in output
    assert "git status" in output
    assert "1 changed" in output
    assert "src/app.py" in output


def test_transcript_renderer_formats_git_diff_in_compact_mode() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ToolCallRequestedEvent(call_id="call-123456", name="git.diff", arguments={})
    )
    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-123456",
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
    assert "tool call" not in output
    assert "git diff" in output
    assert "(+1 -1)" in output
    assert "+new" in output


def test_transcript_renderer_formats_edit_result_summary() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console)

    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="patch.apply",
            output=json.dumps(
                {
                    "path": "src/app.py",
                    "replacements": 1,
                    "changed_line_ranges": [{"start": 4, "end": 5}],
                    "diff": "--- a/src/app.py\n+++ b/src/app.py\n@@\n-old\n+new\n",
                }
            ),
        )
    )

    output = console.export_text()
    assert "edited" in output
    assert "src/app.py" in output
    assert "(+1 -1)" in output
    assert "lines=4-5" in output
    assert "+new" in output


def test_transcript_renderer_verbose_also_formats_edit_diff() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, events_mode="verbose")

    renderer.render(
        ToolCallCompletedEvent(
            call_id="call-1",
            name="filesystem.replace",
            output=json.dumps(
                {
                    "path": "src/app.py",
                    "replacements": 1,
                    "changed_line_ranges": [{"start": 4, "end": 4}],
                    "diff": "--- a/src/app.py\n+++ b/src/app.py\n@@\n-old\n+new\n",
                }
            ),
        )
    )

    output = console.export_text()
    assert "edited" in output
    assert "(+1 -1)" in output
    assert "lines=4" in output
    assert "+new" in output


def test_transcript_renderer_formats_work_state_and_context_semantics() -> None:
    console = Console(record=True, width=160)
    renderer = TranscriptRenderer(console, transcript_style="dense")

    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-task",
            name="task.list",
            arguments={"status": "open"},
        )
    )
    for name, payload in (
        (
            "task.list",
            {
                "tasks": [
                    {
                        "id": "task-alpha",
                        "status": "in_progress",
                        "title": "Theme Python tool renders",
                    }
                ]
            },
        ),
        (
            "decision.create",
            {
                "decision": {
                    "id": "decision-alpha",
                    "status": "active",
                    "title": "Use semantic renderers",
                    "decision": "Keep rendering in the interface layer.",
                }
            },
        ),
        (
            "memory.search",
            {
                "memories": [
                    {
                        "id": "memory-alpha",
                        "scope": "repo",
                        "kind": "note",
                        "text": "Prefer bounded summaries.",
                    }
                ]
            },
        ),
        (
            "plan.create",
            {
                "plan": {
                    "id": "plan-alpha",
                    "status": "draft",
                    "prompt": "Implement renderer parity",
                    "steps": [
                        {
                            "index": 1,
                            "title": "Add semantic summaries",
                            "requires_mutation": True,
                        }
                    ],
                }
            },
        ),
        (
            "goal.update",
            {
                "goal": {
                    "id": "goal-alpha",
                    "status": "complete",
                    "objective": "Finish Python renderer parity",
                    "summary": "All families have summaries.",
                    "iteration_budget": 5,
                    "iterations_completed": 2,
                }
            },
        ),
        (
            "context.show",
            {
                "status": {
                    "session_id": "session-alpha",
                    "message_count": 7,
                    "token_estimate": 1234,
                    "context_window_tokens": 8000,
                    "compacted": True,
                    "auto_compaction": True,
                    "latest_snapshot_id": "snapshot-alpha",
                }
            },
        ),
    ):
        renderer.render(_completed(name, payload))

    output = console.export_text()
    assert "tool call task.list" not in output
    for want in (
        "tasks 1",
        "in_progress task-alp Theme Python tool renders",
        "decision active decision Use semantic renderers",
        "memory search 1",
        "repo/note memory-a Prefer bounded summaries.",
        "plan draft plan-alp steps=1",
        "goal complete goal-alp iterations=2/5",
        "context session=session- tokens=1234/8000 messages=7 compacted=true auto=true",
        "latest_snapshot=snapshot",
    ):
        assert want in output


def test_transcript_renderer_formats_repo_skill_web_discovery_and_integrations() -> None:
    console = Console(record=True, width=180)
    renderer = TranscriptRenderer(console, transcript_style="dense")

    for name, payload in (
        (
            "repo.map",
            {
                "root": ".",
                "files": [{"path": "main.py", "size": 42, "extension": ".py"}],
                "extension_counts": {".py": 1},
            },
        ),
        (
            "repo.symbol_search",
            {
                "symbols": [
                    {"path": "main.py", "line": 12, "kind": "def", "name": "render"}
                ]
            },
        ),
        (
            "agent.list",
            {
                "agents": [
                    {
                        "id": "agent-alpha",
                        "status": "queued",
                        "role": "subagent_default",
                        "task": "Check renderer output",
                    }
                ]
            },
        ),
        (
            "skill.resource.read",
            {
                "resource": {
                    "path": "references/guide.md",
                    "size": 11,
                    "content": "alpha\nbeta",
                }
            },
        ),
        (
            "web.fetch",
            {
                "url": "https://example.test/docs",
                "status_code": 200,
                "content_type": "text/plain",
                "content": "hello docs",
            },
        ),
        (
            "web.search",
            {
                "query": "renderer coverage",
                "search_provider": "searxng",
                "results": [
                    {"title": "Renderer Guide", "url": "https://example.test/render"}
                ],
            },
        ),
        (
            "mcp.servers",
            {
                "configured": True,
                "message": "Configured MCP discovery only",
                "servers": [
                    {
                        "name": "docs",
                        "allowed_tools": ["search"],
                        "env_keys": ["TOKEN"],
                    }
                ],
            },
        ),
        ("mcp.call", {"result": {"id": "item-1", "title": "MCP item"}}),
        (
            "tool.search",
            {
                "tools": [
                    {
                        "name": "repo.map",
                        "risk": "low",
                        "approval_required": False,
                        "description": "Return map",
                    }
                ]
            },
        ),
        ("trace.show", {"available": True, "events": [{"event": "tool.completed"}]}),
        (
            "openapi.demo.getitem",
            {
                "status_code": 200,
                "result": [
                    {
                        "id": "item-1",
                        "title": "Demo item",
                        "url": "https://example.test/item-1",
                    }
                ],
            },
        ),
        ("demo.pack_tool", {"status": "ok", "items": ["one", "two"]}),
    ):
        renderer.render(_completed(name, payload))

    output = console.export_text()
    for want in (
        "repo map . files=1",
        "extensions .py=1",
        "symbols 1",
        "main.py:12 def render",
        "subagents 1",
        "queued agent-al role=subagent_default Check renderer output",
        "resource read references/guide.md 11 bytes",
        "fetch status=200 bytes=10 type=text/plain url=https://example.test/docs",
        "web search provider=searxng results=1 query=renderer coverage",
        "mcp servers=1 configured=true",
        "tool result mcp.call exit=0 keys=result call_id=call-mcp",
        "result.keys=id,title",
        "catalog 1",
        "trace events=1 available=true",
        "openapi openapi.demo.getitem status=200 exit=0 items=1",
        "tool result demo.pack_tool exit=0 keys=items,status call_id=call-dem",
    ):
        assert want in output


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
    renderer = TranscriptRenderer(console, events_mode="verbose")

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


def test_transcript_renderer_verbose_dumps_model_request() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, events_mode="verbose")

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
    assert "model request" in output
    assert '"instructions": "system prompt text"' in output
    assert '"content": "hello"' in output
    assert '"memory.create"' in output


def test_transcript_renderer_compact_shows_context_compaction() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, events_mode="compact")

    renderer.render(
        ContextPreparedEvent(
            turn=0,
            model="demo-model",
            token_estimate=1_200,
            original_token_estimate=24_000,
            context_window_tokens=32_768,
            threshold_tokens=22_937,
            target_tokens=14_745,
            snapshot_id="snapshot-123456",
            compacted=True,
            snapshot_created=True,
        )
    )

    output = console.export_text()
    assert "auto context compaction" in output
    assert "snapshot=snapshot" in output
    assert "original=24,000 -> effective=1,200" in output


def test_transcript_renderer_compact_hides_reused_context_snapshot() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, events_mode="compact")

    renderer.render(
        ContextPreparedEvent(
            turn=0,
            model="demo-model",
            token_estimate=1_200,
            original_token_estimate=24_000,
            context_window_tokens=32_768,
            threshold_tokens=22_937,
            target_tokens=14_745,
            snapshot_id="snapshot-123456",
            compacted=True,
            snapshot_created=False,
        )
    )

    assert "context snapshot reused" not in console.export_text()


def test_transcript_renderer_verbose_shows_reused_context_snapshot() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, events_mode="verbose")

    renderer.render(
        ContextPreparedEvent(
            turn=0,
            model="demo-model",
            token_estimate=1_200,
            original_token_estimate=24_000,
            context_window_tokens=32_768,
            threshold_tokens=22_937,
            target_tokens=14_745,
            snapshot_id="snapshot-123456",
            compacted=True,
            snapshot_created=False,
        )
    )

    assert "context snapshot reused" in console.export_text()


def test_transcript_renderer_compact_hides_uncompacted_context_preparation() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, events_mode="compact")

    renderer.render(
        ContextPreparedEvent(
            turn=0,
            model="demo-model",
            token_estimate=1_200,
            original_token_estimate=1_200,
            context_window_tokens=32_768,
            threshold_tokens=22_937,
            target_tokens=14_745,
        )
    )

    assert console.export_text() == ""


def test_transcript_renderer_compact_hides_model_request() -> None:
    console = Console(record=True, width=120)
    renderer = TranscriptRenderer(console, events_mode="compact")

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


def test_transcript_renderer_stops_activity_before_manual_approval_prompt() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, events_mode="off")

    renderer.begin_run(activity_context="mode=single model=primary:demo")
    renderer.render(
        RiskAssessmentEvent(
            call_id="call-2",
            tool="shell.run",
            risk_level="medium",
            summary="Lists active processes.",
            recommended_decision="requires_approval",
            model_role="risk_evaluator",
            profile_name="primary",
        )
    )
    assert renderer.activity_label == (
        "Reviewing risk for shell.run... | mode=single model=primary:demo"
    )

    renderer.render(ApprovalRequestedEvent(call_id="call-2", reason="Needs permission."))

    assert renderer.activity_label is None
    assert "approval requested" not in console.export_text()


def test_transcript_renderer_stops_activity_before_user_ask_prompt() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, events_mode="off")

    renderer.begin_run(activity_context="mode=plan model=primary:demo")
    renderer.render(
        ToolCallRequestedEvent(
            call_id="call-ask",
            name="user.ask",
            arguments={"question": "Which path?"},
        )
    )

    assert renderer.activity_label is None
    assert "Using user.ask" not in console.export_text()


def test_transcript_renderer_compact_skips_sticky_approval_requested_block() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(console, events_mode="compact")

    renderer.render(ApprovalRequestedEvent(call_id="call-1", reason="Needs permission."))

    assert "approval requested" not in console.export_text()


def test_transcript_renderer_buffered_delta_keeps_activity_until_final_output() -> None:
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
    assert renderer.activity_label == "Using filesystem.read... | mode=single model=primary:demo"
    assert "hello" not in console.export_text()
    renderer.end_run()

    output = console.export_text()
    assert "hello" in output
    assert "filesystem.read" not in output
    assert renderer.activity_label is None


def test_transcript_renderer_raw_streaming_delta_stops_activity_before_output() -> None:
    console = Console(record=True, width=100)
    renderer = TranscriptRenderer(
        console,
        events_mode="off",
        render_streamed_markdown=False,
    )

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


def _completed(name: str, payload: dict[str, object]) -> ToolCallCompletedEvent:
    return ToolCallCompletedEvent(
        call_id=f"call-{name.split('.', 1)[0]}",
        name=name,
        output=json.dumps(payload),
    )
