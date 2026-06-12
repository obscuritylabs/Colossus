from collections.abc import AsyncIterator
from typing import Any

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.approvals import AllowAllApprovalHandler, DenyByDefaultApprovalHandler
from colossus.application.defaults import default_agent
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.errors import PolicyDeniedError
from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    FinalOutputEvent,
    RunEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.requests import AgentRunRequest, ModelRequest
from colossus.domain.tools import ToolCall, ToolPermission, ToolSpec


class SingleToolProvider:
    name = "single-tool"

    def __init__(self, name: str, arguments: dict[str, object]) -> None:
        self._name = name
        self._arguments = arguments

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            yield FinalOutputEvent(text="done")
            return
        yield ToolCallRequestedEvent(call_id="call-1", name=self._name, arguments=self._arguments)


class FailingApprovalHandler:
    async def approve(self, call: Any, decision: Any) -> bool:
        raise AssertionError("approval handler should not be called")


def _orchestrator(tmp_path, provider, approval_handler) -> AgentOrchestrator:
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    return AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=approval_handler,
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )


@pytest.mark.asyncio
async def test_read_only_tool_runs_without_approval(tmp_path) -> None:
    (tmp_path / "note.txt").write_text("hello", encoding="utf-8")
    orchestrator = _orchestrator(
        tmp_path,
        SingleToolProvider("filesystem.read", {"path": "note.txt"}),
        DenyByDefaultApprovalHandler(),
    )

    result = await orchestrator.run(AgentRunRequest(prompt="read", agent=default_agent()))

    assert result.final_output == "done"


@pytest.mark.asyncio
async def test_write_tool_emits_approval_event_when_allowed(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=SingleToolProvider(
            "filesystem.write",
            {"path": "note.txt", "content": "hello", "mode": "create"},
        ),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    await orchestrator.run(AgentRunRequest(prompt="write", agent=default_agent()))
    events = await state.list_events("run-1")

    assert any(isinstance(event, ApprovalRequestedEvent) for event in events)
    assert (tmp_path / "note.txt").read_text(encoding="utf-8") == "hello"


@pytest.mark.asyncio
async def test_full_access_auto_approves_without_prompt_event(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=SingleToolProvider(
            "filesystem.write",
            {"path": "note.txt", "content": "hello", "mode": "create"},
        ),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=FailingApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        auto_approve_required_tools=True,
    )

    await orchestrator.run(AgentRunRequest(prompt="write", agent=default_agent()))
    events = await state.list_events("run-1")
    audit = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")

    assert any(isinstance(event, ApprovalAutoGrantedEvent) for event in events)
    assert not any(isinstance(event, ApprovalRequestedEvent) for event in events)
    assert (tmp_path / "note.txt").read_text(encoding="utf-8") == "hello"
    assert "tool.auto_approved" in audit
    assert "full-access" in audit


@pytest.mark.asyncio
async def test_denied_approval_stops_execution_and_audits(tmp_path) -> None:
    orchestrator = _orchestrator(
        tmp_path,
        SingleToolProvider(
            "filesystem.write",
            {"path": "note.txt", "content": "hello", "mode": "create"},
        ),
        DenyByDefaultApprovalHandler(),
    )

    with pytest.raises(PolicyDeniedError):
        await orchestrator.run(AgentRunRequest(prompt="write", agent=default_agent()))

    audit = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
    assert "tool.denied" in audit
    assert not (tmp_path / "note.txt").exists()


def test_default_policy_gates_network_and_high_risk_tools() -> None:
    policy = DefaultPolicyEngine()
    network_spec = ToolSpec(
        name="web.search",
        description="Search",
        input_schema={"type": "object"},
        permissions=ToolPermission(network="allow", risk="high"),
    )
    high_risk_spec = ToolSpec(
        name="build.run",
        description="Build",
        input_schema={"type": "object"},
        permissions=ToolPermission(risk="high"),
    )

    network_decision = policy.decide_tool_call(
        network_spec,
        ToolCall(call_id="1", name="web.search", arguments={}),
    )
    high_risk_decision = policy.decide_tool_call(
        high_risk_spec,
        ToolCall(call_id="2", name="build.run", arguments={}),
    )

    assert network_decision.decision == "requires_approval"
    assert high_risk_decision.decision == "requires_approval"


@pytest.mark.asyncio
async def test_invalid_tool_arguments_return_tool_error_before_approval(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=SingleToolProvider(
            "filesystem.write",
            {"path": "note.txt", "content": "hello"},
        ),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    result = await orchestrator.run(AgentRunRequest(prompt="write", agent=default_agent()))

    assert result.final_output == "done"
    events = await state.list_events("run-1")
    assert not any(isinstance(event, ApprovalRequestedEvent) for event in events)
    completed = next(event for event in events if isinstance(event, ToolCallCompletedEvent))
    assert completed.exit_code == 1
    assert "invalid_arguments" in completed.output
