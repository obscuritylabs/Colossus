import asyncio
import json
from pathlib import Path

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.subagents import SubagentService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.requests import AgentRunResult
from colossus.domain.subagents import SubagentJob
from colossus.domain.tools import ToolCall


@pytest.mark.asyncio
async def test_subagent_tools_create_and_read_durable_jobs(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = SubagentService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        subagent_service=service,
    )
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)

    created = await executor.execute(
        ToolCall(
            call_id="call-1",
            name="agent.delegate",
            arguments={
                "id": "agent-1",
                "session_id": "session-1",
                "parent_run_id": "run-1",
                "parent_call_id": "call-1",
                "role": "reviewer",
                "task": "check tests",
            },
        )
    )
    listed = await executor.execute(
        ToolCall(
            call_id="call-2",
            name="agent.list",
            arguments={"session_id": "session-1"},
        )
    )
    result = await executor.execute(
        ToolCall(
            call_id="call-3",
            name="agent.result",
            arguments={"id": "agent-1"},
        )
    )

    assert json.loads(created.output)["agent"]["status"] == "queued"
    assert json.loads(listed.output)["agents"][0]["id"] == "agent-1"
    assert json.loads(result.output)["agent"]["task"] == "check tests"


@pytest.mark.asyncio
async def test_subagent_service_runs_jobs_with_bounded_concurrency(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = SubagentService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        max_concurrent=2,
    )
    active = 0
    max_seen = 0

    async def runner(job: SubagentJob) -> AgentRunResult:
        nonlocal active, max_seen
        active += 1
        max_seen = max(max_seen, active)
        await asyncio.sleep(0.01)
        active -= 1
        return AgentRunResult(
            run_id=f"child-{job.id}",
            final_output=f"done {job.task}",
            events_recorded=1,
            session_id=job.child_session_id,
        )

    service.set_runner(runner)
    for index in range(4):
        await service.create_job(
            session_id="session-1",
            parent_run_id="run-1",
            parent_call_id=f"call-{index}",
            task=f"task {index}",
            job_id=f"agent-{index}",
        )

    status = await service.drain()
    jobs = await state.list_subagent_jobs(session_id="session-1")

    assert max_seen == 2
    assert status.completed == 4
    assert status.runner_configured is True
    assert {job.status for job in jobs} == {"completed"}
    assert all(job.child_run_id for job in jobs)


@pytest.mark.asyncio
async def test_subagent_service_reports_status_and_resumes_jobs(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    failed = SubagentJob(
        id="agent-1",
        session_id="session-1",
        parent_run_id="run-1",
        parent_call_id="call-1",
        task="retry work",
        status="failed",
        child_session_id="session-1:subagent:agent-1",
        child_run_id="child-1",
        final_output="old output",
        error="boom",
    )
    await state.save_subagent_job(failed)
    service = SubagentService(state, JsonlAuditSink(tmp_path / "audit.jsonl"), max_concurrent=3)

    before = await service.queue_status(session_id="session-1")
    resumed = await service.resume_job("agent-1")
    after = await service.queue_status(session_id="session-1")

    assert before.failed == 1
    assert before.runner_configured is False
    assert resumed.status == "queued"
    assert resumed.child_run_id is None
    assert resumed.final_output == ""
    assert resumed.error == ""
    assert after.queued == 1
    assert after.max_concurrent == 3


@pytest.mark.asyncio
async def test_subagent_service_drain_timeout_does_not_cancel_running_job(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = SubagentService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    release = asyncio.Event()

    async def runner(job: SubagentJob) -> AgentRunResult:
        await release.wait()
        return AgentRunResult(
            run_id=f"child-{job.id}",
            final_output="done",
            events_recorded=1,
            session_id=job.child_session_id,
        )

    service.set_runner(runner)
    await service.create_job(
        session_id="session-1",
        parent_run_id="run-1",
        parent_call_id="call-1",
        task="wait",
        job_id="agent-1",
    )
    await asyncio.sleep(0)

    status = await service.drain(timeout_seconds=0)
    running = await service.get_job("agent-1")
    release.set()
    final_status = await service.drain()

    assert status.running == 1
    assert running.status == "running"
    assert final_status.completed == 1


@pytest.mark.asyncio
async def test_subagent_service_marks_stale_running_jobs_interrupted(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    stale = SubagentJob(
        id="agent-1",
        session_id="session-1",
        parent_run_id="run-1",
        parent_call_id="call-1",
        task="old work",
        status="running",
        child_session_id="session-1:subagent:agent-1",
    )
    await state.save_subagent_job(stale)

    service = SubagentService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    await service.start()
    updated = await service.get_job("agent-1")

    assert updated.status == "interrupted"
    assert "exited" in updated.error


def test_child_tool_catalog_excludes_nested_delegate(tmp_path: Path) -> None:
    specs, handlers = create_builtin_tools(
        Workspace(tmp_path),
        include_agent_delegate=False,
    )
    names = {spec.name for spec in specs}

    assert "agent.delegate" not in names
    assert "agent.delegate" not in handlers
    assert "agent.result" in names
    assert "agent.list" in names


def test_subagent_tool_schemas_mark_injected_context_fields(tmp_path: Path) -> None:
    specs, _ = create_builtin_tools(Workspace(tmp_path))
    by_name = {spec.name: spec for spec in specs}

    delegate_properties = by_name["agent.delegate"].input_schema["properties"]
    result_properties = by_name["agent.result"].input_schema["properties"]
    list_properties = by_name["agent.list"].input_schema["properties"]

    assert "task" in delegate_properties
    assert delegate_properties["role"]["x-colossus-provider-hidden"] is True
    assert delegate_properties["session_id"]["x-colossus-injected"] is True
    assert delegate_properties["parent_run_id"]["x-colossus-injected"] is True
    assert delegate_properties["parent_call_id"]["x-colossus-injected"] is True
    assert result_properties["session_id"]["x-colossus-injected"] is True
    assert list_properties["session_id"]["x-colossus-injected"] is True
