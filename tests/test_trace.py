import json

from rich.console import Console

from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ContextPreparedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
    ResearchProgressEvent,
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


def test_trace_renderer_compact_formats_research_progress() -> None:
    console = Console(record=True, width=200)
    renderer = RichRunEventRenderer(console)

    renderer.render(
        ResearchProgressEvent(
            research_id="research-1",
            phase="collecting",
            action="web",
            status="completed",
            message="Web search returned 4 result(s).",
            query="deep research progress telemetry",
            source_kind="web",
            current=2,
            total=3,
            sources_collected=7,
            details={"results": 4, "added": 2, "configured": True, "approved": True},
        )
    )

    output = console.export_text()
    assert "research progress collecting web completed 2/3" in output
    assert "results=4" in output
    assert "added=2" in output
    assert "sources=7" in output
    assert "configured=true" in output
    assert 'query="deep research progress telemetry"' in output


def test_trace_renderer_verbose_formats_research_progress_details() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="verbose")

    renderer.render(
        ResearchProgressEvent(
            research_id="research-1",
            phase="synthesis",
            action="deterministic_fallback",
            status="completed",
            message="Built deterministic cited research report.",
            sources_collected=2,
            claims_collected=2,
            details={"report_chars": 1200, "labels": ["R1", "R2"]},
        )
    )

    output = console.export_text()
    assert "research progress synthesis deterministic_fallback completed" in output
    assert "report_chars=1200" in output
    assert "details " in output
    assert '"labels": ["R1", "R2"]' in output


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


def test_trace_renderer_formats_work_state_and_context_semantics() -> None:
    console = Console(record=True, width=140)
    renderer = RichRunEventRenderer(console)

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


def test_trace_renderer_formats_repo_skill_web_discovery_and_integrations() -> None:
    console = Console(record=True, width=140)
    renderer = RichRunEventRenderer(console)

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
            {"symbols": [{"path": "main.py", "line": 12, "kind": "def", "name": "render"}]},
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
            {"resource": {"path": "references/guide.md", "size": 11, "content": "alpha\nbeta"}},
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
                "results": [{"title": "Renderer Guide", "url": "https://example.test/render"}],
            },
        ),
        (
            "mcp.servers",
            {
                "configured": True,
                "message": "Configured MCP discovery only",
                "servers": [{"name": "docs", "allowed_tools": ["search"], "env_keys": ["TOKEN"]}],
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
                "result": [{"id": "item-1", "title": "Demo item", "url": "https://example.test/item-1"}],
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


def test_trace_renderer_compact_shows_context_compaction() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="compact")

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
    assert "threshold=22,937" in output


def test_trace_renderer_compact_hides_reused_context_snapshot() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="compact")

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

    assert console.export_text() == ""


def test_trace_renderer_verbose_shows_reused_context_snapshot() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="verbose")

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


def test_trace_renderer_compact_hides_uncompacted_context_preparation() -> None:
    console = Console(record=True, width=120)
    renderer = RichRunEventRenderer(console, events_mode="compact")

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


def _completed(name: str, payload: dict[str, object]) -> ToolCallCompletedEvent:
    return ToolCallCompletedEvent(
        call_id=f"call-{name.split('.', 1)[0]}",
        name=name,
        output=json.dumps(payload),
    )
