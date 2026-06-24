from collections.abc import AsyncIterator

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.approvals import AllowAllApprovalHandler
from colossus.application.decisions import DecisionService
from colossus.application.defaults import default_agent
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.errors import ProviderError
from colossus.domain.events import (
    FinalOutputEvent,
    ReasoningSummaryEvent,
    RunEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.messages import AssistantMessage, ToolResultMessage
from colossus.domain.requests import AgentRunRequest, ModelRequest
from colossus.domain.tools import ToolSpec


class ToolThenFinalProvider:
    name = "tool-then-final"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            yield FinalOutputEvent(text="done")
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="echo",
                arguments={"text": "from tool"},
            )


class InvalidToolThenFinalProvider:
    name = "invalid-tool-then-final"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        tool_results = [message for message in request.messages if message.role == "tool"]
        if tool_results:
            assert "invalid_arguments" in tool_results[-1].content
            yield FinalOutputEvent(text="recovered")
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="echo",
                arguments={"text": 123},
            )


class MissingFileReadThenFinalProvider:
    name = "missing-file-read-then-final"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        tool_results = [message for message in request.messages if message.role == "tool"]
        if tool_results:
            assert "execution_error" in tool_results[-1].content
            assert "file not found" in tool_results[-1].content
            yield FinalOutputEvent(text="recovered")
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="filesystem.read",
                arguments={"path": "missing.txt"},
            )


class TaskToolThenFinalProvider:
    name = "task-tool-then-final"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            yield FinalOutputEvent(text="done")
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="task.create",
                arguments={"title": "Track work"},
            )


class AgentDelegateThenFinalProvider:
    name = "agent-delegate-then-final"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            yield FinalOutputEvent(text="done")
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="agent.delegate",
                arguments={"task": "collect data"},
            )


class FailingProvider:
    name = "failing"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        raise AssertionError("provider should not be called")


class EmptyProvider:
    name = "empty"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        if False:
            yield FinalOutputEvent(text="")


class ReasoningOnlyProvider:
    name = "reasoning-only"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        yield ReasoningSummaryEvent(summary="Thinking but no answer.")


@pytest.mark.asyncio
async def test_orchestrator_executes_tool_and_continues(tmp_path) -> None:
    async def echo(arguments: dict[str, object]) -> str:
        return str(arguments["text"])

    spec = ToolSpec(
        name="echo",
        description="Echo",
        input_schema={
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": False,
        },
    )
    registry = InMemoryToolRegistry((spec,))
    provider = ToolThenFinalProvider()
    observed: list[RunEvent] = []
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"echo": echo}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        event_observer=observed.append,
    )

    result = await orchestrator.run(AgentRunRequest(prompt="use a tool", agent=default_agent()))

    assert result.final_output == "done"
    assert result.events_recorded == 3
    second_turn_messages = provider.requests[1].messages
    assert isinstance(second_turn_messages[-2], AssistantMessage)
    assert second_turn_messages[-2].tool_calls[0].call_id == "call-1"
    assert isinstance(second_turn_messages[-1], ToolResultMessage)
    assert second_turn_messages[-1].call_id == "call-1"
    assert [event.type for event in observed] == [
        "tool.call.requested",
        "tool.call.completed",
        "final.output",
    ]


@pytest.mark.asyncio
async def test_orchestrator_rejects_empty_provider_response(tmp_path) -> None:
    orchestrator = AgentOrchestrator(
        provider=EmptyProvider(),
        tool_registry=InMemoryToolRegistry(()),
        tool_executor=FunctionToolExecutor({}, InMemoryToolRegistry(())),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    with pytest.raises(ProviderError, match="no assistant text or tool calls"):
        await orchestrator.run(AgentRunRequest(prompt="hello", agent=default_agent()))


@pytest.mark.asyncio
async def test_orchestrator_rejects_reasoning_only_provider_response(tmp_path) -> None:
    observed: list[RunEvent] = []
    orchestrator = AgentOrchestrator(
        provider=ReasoningOnlyProvider(),
        tool_registry=InMemoryToolRegistry(()),
        tool_executor=FunctionToolExecutor({}, InMemoryToolRegistry(())),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        event_observer=observed.append,
    )

    with pytest.raises(ProviderError, match="choices\\[\\]\\.delta\\.content"):
        await orchestrator.run(AgentRunRequest(prompt="hello", agent=default_agent()))
    assert [event.type for event in observed] == ["reasoning.summary"]


@pytest.mark.asyncio
async def test_orchestrator_injects_session_id_for_task_tools(tmp_path) -> None:
    observed_arguments: dict[str, object] = {}

    async def task_create(arguments: dict[str, object]) -> str:
        observed_arguments.update(arguments)
        return '{"task": {"id": "task-1"}}'

    spec = ToolSpec(
        name="task.create",
        description="Create task",
        input_schema={
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "session_id": {"type": "string"},
            },
            "required": ["title"],
            "additionalProperties": False,
        },
        output_schema={"type": "object"},
    )
    registry = InMemoryToolRegistry((spec,))
    orchestrator = AgentOrchestrator(
        provider=TaskToolThenFinalProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"task.create": task_create}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    result = await orchestrator.run(
        AgentRunRequest(prompt="make tasks", agent=default_agent(), session_id="session-1")
    )

    assert result.final_output == "done"
    assert observed_arguments["session_id"] == "session-1"


@pytest.mark.asyncio
async def test_orchestrator_captures_standalone_key_decision_before_provider(
    tmp_path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    decision_service = DecisionService(state, audit)
    registry = InMemoryToolRegistry(())
    orchestrator = AgentOrchestrator(
        provider=FailingProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=audit,
        run_id_factory=lambda: "run-1",
        decision_service=decision_service,
    )

    result = await orchestrator.run(
        AgentRunRequest(
            prompt="Mvoing forward I want to make sure run tests and lint",
            agent=default_agent(),
            session_id="session-1",
        )
    )

    decisions = await decision_service.list_decisions(session_id="session-1")
    assert result.final_output == f"Noted as key decision {decisions[0].id}."
    assert decisions[0].source == "user"
    assert decisions[0].priority == "high"
    assert decisions[0].decision == "Mvoing forward I want to make sure run tests and lint"
    assert "decision.created" in (tmp_path / "audit.jsonl").read_text(encoding="utf-8")


@pytest.mark.asyncio
async def test_orchestrator_does_not_duplicate_captured_key_decision(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    decision_service = DecisionService(state, audit)
    await decision_service.create_decision(
        session_id="session-1",
        decision_id="kd_existing",
        title="Existing",
        decision="Moving forward always run tests.",
        source="user",
    )
    registry = InMemoryToolRegistry(())
    orchestrator = AgentOrchestrator(
        provider=FailingProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=audit,
        run_id_factory=lambda: "run-1",
        decision_service=decision_service,
    )

    result = await orchestrator.run(
        AgentRunRequest(
            prompt="Moving forward always run tests.",
            agent=default_agent(),
            session_id="session-1",
        )
    )

    assert result.final_output == "Noted as key decision kd_existing."
    assert len(await decision_service.list_decisions(session_id="session-1")) == 1


@pytest.mark.asyncio
@pytest.mark.asyncio
async def test_orchestrator_injects_subagent_context_after_validation(tmp_path) -> None:
    observed_arguments: dict[str, object] = {}

    async def agent_delegate(arguments: dict[str, object]) -> str:
        observed_arguments.update(arguments)
        return '{"agent": {"id": "agent-1", "status": "queued"}}'

    spec = ToolSpec(
        name="agent.delegate",
        description="Queue subagent",
        input_schema={
            "type": "object",
            "properties": {
                "task": {"type": "string"},
                "session_id": {"type": "string", "x-colossus-injected": True},
                "parent_run_id": {"type": "string", "x-colossus-injected": True},
                "parent_call_id": {"type": "string", "x-colossus-injected": True},
            },
            "required": ["task"],
            "additionalProperties": False,
        },
        output_schema={"type": "object"},
    )
    registry = InMemoryToolRegistry((spec,))
    orchestrator = AgentOrchestrator(
        provider=AgentDelegateThenFinalProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"agent.delegate": agent_delegate}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    result = await orchestrator.run(
        AgentRunRequest(prompt="delegate work", agent=default_agent(), session_id="session-1")
    )

    assert result.final_output == "done"
    assert observed_arguments == {
        "task": "collect data",
        "session_id": "session-1",
        "parent_run_id": "run-1",
        "parent_call_id": "call-1",
    }


@pytest.mark.asyncio
async def test_orchestrator_returns_invalid_tool_args_to_model(tmp_path) -> None:
    async def echo(arguments: dict[str, object]) -> str:
        return str(arguments["text"])

    spec = ToolSpec(
        name="echo",
        description="Echo",
        input_schema={
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": False,
        },
    )
    registry = InMemoryToolRegistry((spec,))
    provider = InvalidToolThenFinalProvider()
    observed: list[RunEvent] = []
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"echo": echo}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        event_observer=observed.append,
    )

    result = await orchestrator.run(AgentRunRequest(prompt="use a tool", agent=default_agent()))

    assert result.final_output == "recovered"
    completed = next(event for event in observed if isinstance(event, ToolCallCompletedEvent))
    assert completed.exit_code == 1
    assert "invalid_arguments" in completed.output
    assert [event.type for event in observed] == [
        "tool.call.requested",
        "tool.call.completed",
        "final.output",
    ]


@pytest.mark.asyncio
async def test_orchestrator_returns_filesystem_read_errors_to_model(tmp_path) -> None:
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    observed: list[RunEvent] = []
    orchestrator = AgentOrchestrator(
        provider=MissingFileReadThenFinalProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        event_observer=observed.append,
    )

    result = await orchestrator.run(AgentRunRequest(prompt="read missing", agent=default_agent()))

    assert result.final_output == "recovered"
    completed = next(event for event in observed if isinstance(event, ToolCallCompletedEvent))
    assert completed.exit_code == 1
    assert "execution_error" in completed.output
    assert "file not found" in completed.output
