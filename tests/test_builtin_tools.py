import json
from pathlib import Path

import httpx
import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.roadmap_tools import create_roadmap_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.subprocess_broker import (
    SubprocessBroker,
    SubprocessCommand,
    SubprocessResult,
)
from colossus.adapters.workspace import Workspace
from colossus.application.context import ContextService
from colossus.application.decisions import DecisionService
from colossus.application.memories import MemoryService
from colossus.application.tasks import TaskService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.context import ContextConfig
from colossus.domain.errors import ToolExecutionError
from colossus.domain.messages import UserMessage
from colossus.domain.tools import ToolCall, ToolSpec
from colossus.domain.user_prompts import UserPromptAnswer, UserPromptChoice


def _executor(root: Path) -> FunctionToolExecutor:
    specs, handlers = create_builtin_tools(Workspace(root))
    registry = InMemoryToolRegistry(specs)
    return FunctionToolExecutor(handlers, registry)


class CapturingBroker(SubprocessBroker):
    def __init__(self) -> None:
        self.commands: list[SubprocessCommand] = []

    async def run(self, command: SubprocessCommand) -> SubprocessResult:
        self.commands.append(command)
        return SubprocessResult(exit_code=0, stdout="ok", stderr="")


class FakeUserPromptHandler:
    def __init__(self) -> None:
        self.question = ""
        self.choices: tuple[UserPromptChoice, ...] = ()
        self.allow_freeform = True

    async def ask(
        self,
        *,
        question: str,
        choices: tuple[UserPromptChoice, ...] = (),
        allow_freeform: bool = True,
    ) -> UserPromptAnswer:
        self.question = question
        self.choices = choices
        self.allow_freeform = allow_freeform
        return UserPromptAnswer(answer=choices[0].label, choice_id=choices[0].id)


def test_builtin_tool_catalog_has_handlers_and_roadmap_families(tmp_path: Path) -> None:
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    names = {spec.name for spec in specs}
    shell_spec = next(spec for spec in specs if spec.name == "shell.run")

    assert names == set(handlers)
    assert "task.create" in names
    assert "decision.create" in names
    assert "memory.create" in names
    assert "plan.approve_request" in names
    assert "test.run" in names
    assert "patch.apply" in names
    assert "repo.symbol_search" in names
    assert "agent.delegate" in names
    assert "web.search" not in names
    assert "mcp.call" not in names
    assert "trace.export" in names
    assert "eval.run" in names
    assert "user.ask" not in names
    assert "structured argv" in shell_spec.description
    assert "pipes" in shell_spec.description
    assert specs[-1].name == "echo"


def test_roadmap_extension_tools_are_opt_in(tmp_path: Path) -> None:
    specs, handlers = create_roadmap_tools(
        Workspace(tmp_path),
        CapturingBroker(),
        include_web_search=True,
        include_mcp_call=True,
    )
    names = {spec.name for spec in specs}

    assert "web.search" in names
    assert "web.search" in handlers
    assert "mcp.call" in names
    assert "mcp.call" in handlers


def test_builtin_tool_catalog_includes_user_ask_when_handler_is_wired(tmp_path: Path) -> None:
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        user_prompt_handler=FakeUserPromptHandler(),
    )
    names = {spec.name for spec in specs}
    user_ask = next(spec for spec in specs if spec.name == "user.ask")

    assert "user.ask" in names
    assert "user.ask" in handlers
    assert user_ask.permissions.risk == "low"
    assert user_ask.permissions.mutation is False
    assert user_ask.permissions.working_root_required is False


def test_tool_registry_rejects_duplicate_names() -> None:
    spec = ToolSpec(name="dup.tool", description="duplicate", input_schema={"type": "object"})

    with pytest.raises(ValueError, match="Duplicate tool names"):
        InMemoryToolRegistry((spec, spec))


@pytest.mark.asyncio
async def test_user_ask_tool_returns_structured_answer(tmp_path: Path) -> None:
    handler = FakeUserPromptHandler()
    specs, handlers = create_builtin_tools(Workspace(tmp_path), user_prompt_handler=handler)
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    result = await executor.execute(
        ToolCall(
            call_id="ask-1",
            name="user.ask",
            arguments={
                "question": "Pick a path",
                "choices": [
                    {
                        "id": "minimal",
                        "label": "Minimal",
                        "description": "Smallest useful change",
                    }
                ],
                "allow_freeform": False,
            },
        )
    )

    assert json.loads(result.output) == {"answer": "Minimal", "choice_id": "minimal"}
    assert handler.question == "Pick a path"
    assert handler.choices[0].id == "minimal"
    assert handler.allow_freeform is False


@pytest.mark.asyncio
async def test_user_ask_tool_requires_choices_when_freeform_is_disabled(tmp_path: Path) -> None:
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        user_prompt_handler=FakeUserPromptHandler(),
    )
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    with pytest.raises(ToolExecutionError, match="requires choices"):
        await executor.execute(
            ToolCall(
                call_id="ask-1",
                name="user.ask",
                arguments={"question": "Pick a path", "allow_freeform": False},
            )
        )


@pytest.mark.asyncio
async def test_filesystem_list_read_and_search(tmp_path: Path) -> None:
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "example.txt").write_text("alpha\nneedle\nomega\n", encoding="utf-8")
    executor = _executor(tmp_path)

    listed = await executor.execute(
        ToolCall(call_id="1", name="filesystem.list", arguments={"path": "src"})
    )
    read = await executor.execute(
        ToolCall(
            call_id="2",
            name="filesystem.read",
            arguments={"path": "src/example.txt", "start_line": 2, "max_lines": 1},
        )
    )
    searched = await executor.execute(
        ToolCall(
            call_id="3",
            name="filesystem.search",
            arguments={
                "path": "src",
                "pattern": "needle",
                "regex": False,
                "max_matches": 5,
            },
        )
    )

    assert json.loads(listed.output)["entries"][0]["path"] == "src/example.txt"
    assert json.loads(read.output)["content"] == "needle"
    assert json.loads(searched.output)["matches"][0]["path"] == "src/example.txt"


@pytest.mark.asyncio
async def test_filesystem_write_and_replace_semantics(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    written = await executor.execute(
        ToolCall(
            call_id="1",
            name="filesystem.write",
            arguments={"path": "note.txt", "content": "hello hello", "mode": "create"},
        )
    )
    replaced = await executor.execute(
        ToolCall(
            call_id="2",
            name="filesystem.replace",
            arguments={"path": "note.txt", "old": "hello", "new": "hi", "replace_all": True},
        )
    )

    assert json.loads(written.output)["path"] == "note.txt"
    assert json.loads(replaced.output)["replacements"] == 2
    assert (tmp_path / "note.txt").read_text(encoding="utf-8") == "hi hi"


@pytest.mark.asyncio
async def test_filesystem_replace_rejects_ambiguous_single_replace(tmp_path: Path) -> None:
    (tmp_path / "note.txt").write_text("hello hello", encoding="utf-8")
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError):
        await executor.execute(
            ToolCall(
                call_id="1",
                name="filesystem.replace",
                arguments={"path": "note.txt", "old": "hello", "new": "hi"},
            )
        )


@pytest.mark.asyncio
async def test_filesystem_read_rejects_binary_looking_file(tmp_path: Path) -> None:
    (tmp_path / "blob.bin").write_bytes(b"abc\x00def")
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="Binary-looking"):
        await executor.execute(
            ToolCall(call_id="1", name="filesystem.read", arguments={"path": "blob.bin"})
        )


@pytest.mark.asyncio
async def test_filesystem_read_missing_file_returns_tool_error(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="file not found"):
        await executor.execute(
            ToolCall(call_id="1", name="filesystem.read", arguments={"path": "missing.txt"})
        )


@pytest.mark.asyncio
async def test_filesystem_tools_reject_paths_outside_workspace(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="escapes workspace"):
        await executor.execute(
            ToolCall(call_id="1", name="filesystem.read", arguments={"path": "../outside.txt"})
        )


@pytest.mark.asyncio
async def test_filesystem_write_create_refuses_existing_file(tmp_path: Path) -> None:
    (tmp_path / "note.txt").write_text("existing", encoding="utf-8")
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="refuses to overwrite"):
        await executor.execute(
            ToolCall(
                call_id="1",
                name="filesystem.write",
                arguments={"path": "note.txt", "content": "new", "mode": "create"},
            )
        )


@pytest.mark.asyncio
async def test_tool_executor_rejects_schema_errors_and_unknown_tools(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="Invalid arguments"):
        await executor.execute(
            ToolCall(call_id="1", name="filesystem.read", arguments={"path": 123})
        )
    with pytest.raises(ToolExecutionError, match="Unknown tool"):
        await executor.execute(ToolCall(call_id="2", name="missing.tool", arguments={}))


@pytest.mark.asyncio
async def test_shell_run_rejects_empty_argv_and_shell_wrappers(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError):
        await executor.execute(ToolCall(call_id="1", name="shell.run", arguments={"argv": []}))
    with pytest.raises(ToolExecutionError):
        await executor.execute(
            ToolCall(call_id="2", name="shell.run", arguments={"argv": ["bash", "-lc", "pwd"]})
        )


@pytest.mark.asyncio
async def test_git_status_returns_structured_entries(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    await executor.execute(
        ToolCall(call_id="1", name="shell.run", arguments={"argv": ["git", "init"]})
    )
    (tmp_path / "new.txt").write_text("new", encoding="utf-8")

    result = await executor.execute(ToolCall(call_id="2", name="git.status", arguments={}))

    assert json.loads(result.output)["entries"][0]["path"] == "new.txt"


@pytest.mark.asyncio
async def test_task_plan_and_agent_tools_track_runtime_state(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    task_created = await executor.execute(
        ToolCall(
            call_id="1",
            name="task.create",
            arguments={"id": "task-1", "title": "Map tools"},
        )
    )
    task_updated = await executor.execute(
        ToolCall(
            call_id="2",
            name="task.update",
            arguments={"id": "task-1", "status": "completed"},
        )
    )
    plan_created = await executor.execute(
        ToolCall(
            call_id="3",
            name="plan.create",
            arguments={"id": "plan-1", "prompt": "ship", "steps": ["one", "two"]},
        )
    )
    plan_shown = await executor.execute(
        ToolCall(call_id="4", name="plan.show", arguments={"id": "plan-1"})
    )
    agent_created = await executor.execute(
        ToolCall(
            call_id="5",
            name="agent.delegate",
            arguments={"id": "agent-1", "role": "reviewer", "task": "check tests"},
        )
    )
    agent_listed = await executor.execute(ToolCall(call_id="6", name="agent.list", arguments={}))

    assert json.loads(task_created.output)["task"]["title"] == "Map tools"
    assert json.loads(task_updated.output)["task"]["status"] == "completed"
    assert json.loads(plan_created.output)["plan"]["steps"][1]["title"] == "two"
    assert json.loads(plan_shown.output)["plan"]["id"] == "plan-1"
    assert json.loads(agent_created.output)["agent"]["status"] == "completed"
    assert json.loads(agent_listed.output)["agents"][0]["id"] == "agent-1"


@pytest.mark.asyncio
async def test_task_tools_persist_session_state_when_service_is_available(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    task_service = TaskService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    specs, handlers = create_builtin_tools(Workspace(tmp_path), task_service=task_service)
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    await executor.execute(
        ToolCall(
            call_id="1",
            name="task.create",
            arguments={
                "id": "task-1",
                "session_id": "session-1",
                "title": "Persist task",
            },
        )
    )
    await executor.execute(
        ToolCall(
            call_id="2",
            name="task.update",
            arguments={
                "id": "task-1",
                "session_id": "session-1",
                "status": "completed",
            },
        )
    )

    reloaded = SQLiteStateStore(tmp_path / "state.sqlite3")
    tasks = await reloaded.list_tasks(session_id="session-1")
    assert len(tasks) == 1
    assert tasks[0].title == "Persist task"
    assert tasks[0].status == "completed"


@pytest.mark.asyncio
async def test_decision_tools_persist_session_state_when_service_is_available(
    tmp_path: Path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    decision_service = DecisionService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        decision_service=decision_service,
    )
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    created = await executor.execute(
        ToolCall(
            call_id="1",
            name="decision.create",
            arguments={
                "id": "kd_1",
                "session_id": "session-1",
                "title": "Durable commitments",
                "decision": "Key decisions are durable commitments, not memories.",
                "priority": "critical",
            },
        )
    )
    listed = await executor.execute(
        ToolCall(
            call_id="2",
            name="decision.list",
            arguments={"session_id": "session-1"},
        )
    )
    superseded = await executor.execute(
        ToolCall(
            call_id="3",
            name="decision.supersede",
            arguments={
                "id": "kd_1",
                "session_id": "session-1",
                "title": "Injected commitments",
                "decision": "Active key decisions are injected before snapshots.",
                "priority": "high",
            },
        )
    )
    archived = await executor.execute(
        ToolCall(
            call_id="4",
            name="decision.archive",
            arguments={
                "id": json.loads(superseded.output)["decision"]["id"],
                "session_id": "session-1",
            },
        )
    )

    assert json.loads(created.output)["decision"]["priority"] == "critical"
    assert json.loads(listed.output)["decisions"][0]["id"] == "kd_1"
    assert json.loads(superseded.output)["decision"]["supersedes"] == "kd_1"
    assert json.loads(archived.output)["decision"]["status"] == "archived"
    all_decisions = await state.list_decisions(session_id="session-1", status=None)
    assert {decision.status for decision in all_decisions} == {"superseded", "archived"}


@pytest.mark.asyncio
async def test_memory_tools_persist_search_and_notice_when_service_is_available(
    tmp_path: Path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    memory_service = MemoryService(state, JsonlAuditSink(tmp_path / "audit.jsonl"), state)
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        memory_service=memory_service,
    )
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    created = await executor.execute(
        ToolCall(
            call_id="1",
            name="memory.create",
            arguments={
                "id": "mem_1",
                "session_id": "session-1",
                "scope": "repo",
                "kind": "preference",
                "text": "Always run pytest and ruff before declaring completion.",
            },
        )
    )
    searched = await executor.execute(
        ToolCall(
            call_id="2",
            name="memory.search",
            arguments={
                "query": "pytest ruff",
                "session_id": "session-1",
            },
        )
    )
    superseded = await executor.execute(
        ToolCall(
            call_id="3",
            name="memory.supersede",
            arguments={
                "id": "mem_1",
                "session_id": "session-1",
                "text": "Run pytest, ruff, and mypy before declaring completion.",
            },
        )
    )
    archived = await executor.execute(
        ToolCall(
            call_id="4",
            name="memory.archive",
            arguments={"id": json.loads(superseded.output)["memory"]["id"]},
        )
    )

    created_payload = json.loads(created.output)
    assert created_payload["notice"] == "Saved memory mem_1 [repo/preference]"
    assert json.loads(searched.output)["memories"][0]["id"] == "mem_1"
    assert json.loads(superseded.output)["memory"]["supersedes"] == "mem_1"
    assert json.loads(archived.output)["memory"]["status"] == "archived"
    all_memories = await state.list_memories(status=None)
    assert {memory.status for memory in all_memories} == {"superseded", "archived"}


@pytest.mark.asyncio
async def test_plan_approval_request_updates_plan_after_policy_allows_it(tmp_path: Path) -> None:
    executor = _executor(tmp_path)
    await executor.execute(
        ToolCall(
            call_id="1",
            name="plan.create",
            arguments={"id": "plan-1", "prompt": "ship"},
        )
    )

    result = await executor.execute(
        ToolCall(call_id="2", name="plan.approve_request", arguments={"id": "plan-1"})
    )

    payload = json.loads(result.output)
    assert payload["approved"] is True
    assert payload["plan"]["status"] == "approved"


@pytest.mark.asyncio
async def test_patch_preview_apply_and_reverse(tmp_path: Path) -> None:
    (tmp_path / "note.txt").write_text("alpha\nbeta\n", encoding="utf-8")
    executor = _executor(tmp_path)
    arguments = {"path": "note.txt", "old": "beta", "new": "gamma"}

    preview = await executor.execute(
        ToolCall(call_id="1", name="patch.preview", arguments=arguments)
    )
    applied = await executor.execute(ToolCall(call_id="2", name="patch.apply", arguments=arguments))
    reversed_patch = await executor.execute(
        ToolCall(call_id="3", name="patch.reverse", arguments=arguments)
    )

    assert "+gamma" in json.loads(preview.output)["diff"]
    assert json.loads(applied.output)["replacements"] == 1
    assert json.loads(reversed_patch.output)["replacements"] == 1
    assert (tmp_path / "note.txt").read_text(encoding="utf-8") == "alpha\nbeta\n"


@pytest.mark.asyncio
async def test_patch_apply_rejects_ambiguous_text(tmp_path: Path) -> None:
    (tmp_path / "note.txt").write_text("alpha alpha", encoding="utf-8")
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="ambiguous"):
        await executor.execute(
            ToolCall(
                call_id="1",
                name="patch.apply",
                arguments={"path": "note.txt", "old": "alpha", "new": "beta"},
            )
        )


@pytest.mark.asyncio
async def test_repo_context_tools_map_symbols_references_and_summary(tmp_path: Path) -> None:
    source = tmp_path / "src" / "sample.py"
    source.parent.mkdir()
    source.write_text(
        "import os\n\nclass Widget:\n    pass\n\ndef build_widget():\n    return Widget()\n",
        encoding="utf-8",
    )
    executor = _executor(tmp_path)

    repo_map = await executor.execute(
        ToolCall(call_id="1", name="repo.map", arguments={"path": "src", "max_files": 20})
    )
    symbols = await executor.execute(
        ToolCall(call_id="2", name="repo.symbol_search", arguments={"pattern": "Widget"})
    )
    references = await executor.execute(
        ToolCall(call_id="3", name="repo.references", arguments={"symbol": "Widget"})
    )
    summary = await executor.execute(
        ToolCall(call_id="4", name="repo.file_summary", arguments={"path": "src/sample.py"})
    )

    assert json.loads(repo_map.output)["files"][0]["path"] == "src/sample.py"
    assert json.loads(symbols.output)["symbols"][0]["name"] == "Widget"
    assert json.loads(references.output)["references"][0]["path"] == "src/sample.py"
    assert json.loads(summary.output)["imports"] == ["import os"]


@pytest.mark.asyncio
async def test_web_fetch_uses_bounded_http_client(tmp_path: Path) -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        assert str(request.url) == "https://example.com"
        return httpx.Response(
            200,
            headers={"content-type": "text/html; charset=utf-8"},
            content=b"abcdef",
            request=request,
        )

    specs, handlers = create_roadmap_tools(
        Workspace(tmp_path),
        CapturingBroker(),
        http_transport=httpx.MockTransport(handler),
    )
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    result = await executor.execute(
        ToolCall(
            call_id="1",
            name="web.fetch",
            arguments={"url": "https://example.com", "max_bytes": 4},
        )
    )

    payload = json.loads(result.output)
    assert payload == {
        "content": "abcd",
        "content_type": "text/html; charset=utf-8",
        "status_code": 200,
        "truncated": True,
        "url": "https://example.com",
    }


@pytest.mark.asyncio
async def test_network_tools_reject_non_http_urls(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match="http:// or https://"):
        await executor.execute(
            ToolCall(call_id="1", name="web.fetch", arguments={"url": "file:///etc/passwd"})
        )


@pytest.mark.asyncio
async def test_mcp_calls_are_disabled_by_default(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    with pytest.raises(ToolExecutionError, match=r"Unknown tool: mcp\.call"):
        await executor.execute(
            ToolCall(
                call_id="1",
                name="mcp.call",
                arguments={"server": "s", "tool": "t", "arguments": {}},
            )
        )

    servers = await executor.execute(ToolCall(call_id="2", name="mcp.servers", arguments={}))
    assert json.loads(servers.output)["configured"] is False


@pytest.mark.asyncio
async def test_opted_in_mcp_calls_require_adapter(tmp_path: Path) -> None:
    specs, handlers = create_roadmap_tools(
        Workspace(tmp_path),
        CapturingBroker(),
        include_mcp_call=True,
    )
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    with pytest.raises(ToolExecutionError, match="MCP calls require"):
        await executor.execute(
            ToolCall(
                call_id="1",
                name="mcp.call",
                arguments={"server": "s", "tool": "t", "arguments": {}},
            )
        )


@pytest.mark.asyncio
async def test_tool_search_uses_registered_catalog(tmp_path: Path) -> None:
    executor = _executor(tmp_path)

    result = await executor.execute(
        ToolCall(call_id="1", name="tool.search", arguments={"query": "patch"})
    )

    names = {item["name"] for item in json.loads(result.output)["tools"]}
    assert {"patch.preview", "patch.apply", "patch.reverse"}.issubset(names)


@pytest.mark.asyncio
async def test_trace_show_and_export(tmp_path: Path) -> None:
    (tmp_path / ".colossus_trace.jsonl").write_text('{"event": "one"}\n', encoding="utf-8")
    executor = _executor(tmp_path)

    shown = await executor.execute(ToolCall(call_id="1", name="trace.show", arguments={}))
    exported = await executor.execute(
        ToolCall(call_id="2", name="trace.export", arguments={"path": "trace-export.json"})
    )

    assert json.loads(shown.output)["events"][0]["event"] == "one"
    assert json.loads(exported.output)["path"] == "trace-export.json"
    assert (tmp_path / "trace-export.json").exists()


@pytest.mark.asyncio
async def test_verification_tools_use_brokered_fixed_commands(tmp_path: Path) -> None:
    broker = CapturingBroker()
    catalog: list[ToolSpec] = []
    specs, handlers = create_roadmap_tools(Workspace(tmp_path), broker, lambda: tuple(catalog))
    catalog.extend(specs)
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    result = await executor.execute(
        ToolCall(
            call_id="1",
            name="test.run",
            arguments={"paths": ["."], "extra_args": ["-q"]},
        )
    )

    assert json.loads(result.output)["stdout"] == "ok"
    assert broker.commands[0].argv == ("uv", "run", "pytest", ".", "-q")


@pytest.mark.asyncio
async def test_context_tools_use_context_service(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ContextService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        config=ContextConfig(model_assisted=False),
        snapshot_id_factory=lambda: "snapshot-1",
    )
    await state.append_message("session-1", "run-1", UserMessage(content="remember this"))
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        context_service=service,
        context_model="model-a",
    )
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    compacted = await executor.execute(
        ToolCall(
            call_id="1",
            name="context.compact",
            arguments={"session_id": "session-1", "model": "model-a"},
        )
    )
    shown = await executor.execute(
        ToolCall(call_id="2", name="context.show", arguments={"session_id": "session-1"})
    )
    snapshots = await executor.execute(
        ToolCall(call_id="3", name="context.snapshots", arguments={"session_id": "session-1"})
    )
    restored = await executor.execute(
        ToolCall(call_id="4", name="context.restore", arguments={"snapshot_id": "snapshot-1"})
    )

    restore_spec = registry.get_spec("context.restore")
    assert json.loads(compacted.output)["snapshot"]["id"] == "snapshot-1"
    assert json.loads(shown.output)["status"]["latest_snapshot_id"] == "snapshot-1"
    assert json.loads(snapshots.output)["snapshots"][0]["id"] == "snapshot-1"
    assert json.loads(restored.output)["restored"] is True
    assert restore_spec is not None
    assert restore_spec.permissions.approval_required is True
    assert restore_spec.permissions.mutation is True
