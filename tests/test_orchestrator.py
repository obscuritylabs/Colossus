from collections.abc import AsyncIterator

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.approvals import AllowAllApprovalHandler
from colossus.application.defaults import default_agent
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.events import (
    FinalOutputEvent,
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
