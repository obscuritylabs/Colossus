import json
from collections.abc import AsyncIterator

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.skills_filesystem import FilesystemSkillRepository
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.approvals import AllowAllApprovalHandler, DenyByDefaultApprovalHandler
from colossus.application.context import ContextService
from colossus.application.decisions import DecisionService
from colossus.application.defaults import default_agent
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.skills import SkillComposer, SkillResolver
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.context import ContextConfig
from colossus.domain.errors import ColossusError, ProviderError
from colossus.domain.events import (
    ErrorEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
    ReasoningSummaryEvent,
    RunEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.messages import AssistantMessage, ToolResultMessage, UserMessage
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


class SkillResourceToolThenFinalProvider:
    name = "skill-resource-tool-then-final"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            yield FinalOutputEvent(text="done")
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="skill.resource.read",
                arguments={"skill": "alpha", "path": "references/guide.md"},
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


class MalformedToolArgumentsThenFinalProvider:
    name = "malformed-tool-arguments-then-final"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        if len(self.requests) == 1:
            raise ProviderError(
                "Provider returned invalid JSON for tool call arguments. "
                "tool=shell_run call_id=call_bad size=935 position=25"
            )
        recovery_messages = [
            message.content
            for message in request.messages
            if isinstance(message, UserMessage)
            and "invalid tool-call arguments" in message.content
        ]
        assert recovery_messages
        assert "tool=shell_run" in recovery_messages[-1]
        yield FinalOutputEvent(text="recovered")


class AlwaysMalformedToolArgumentsProvider:
    name = "always-malformed-tool-arguments"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        if False:
            yield FinalOutputEvent(text="")
        raise ProviderError(
            "Provider returned invalid JSON for tool call arguments. "
            "tool=filesystem_write call_id=call_bad size=1709 position=84"
        )


class ReasoningOnlyProvider:
    name = "reasoning-only"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        yield ReasoningSummaryEvent(summary="Thinking but no answer.")


class CapturingFinalProvider:
    name = "capturing-final"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        yield FinalOutputEvent(text="done")


class ToolSearchThenEchoProvider:
    name = "tool-search-then-echo"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        tool_results = [message for message in request.messages if message.role == "tool"]
        if any(message.name == "echo" for message in tool_results):
            yield FinalOutputEvent(text="done")
        elif any(message.name == "tool.search" for message in tool_results):
            yield ToolCallRequestedEvent(
                call_id="call-2",
                name="echo",
                arguments={"text": "after search"},
            )
        else:
            yield ToolCallRequestedEvent(
                call_id="call-1",
                name="tool.search",
                arguments={"query": "echo"},
            )


class TextToolThenEmptyProvider:
    name = "text-tool-then-empty"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            return
        yield ModelDeltaEvent(text="Reading the file.")
        yield FinalOutputEvent(text="Reading the file.")
        yield ToolCallRequestedEvent(
            call_id="call-1",
            name="echo",
            arguments={"text": "from tool"},
        )


class ShellToolProvider:
    name = "shell-tool"

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        yield ToolCallRequestedEvent(
            call_id="call-1",
            name="shell.run",
            arguments={"argv": ["echo", "hello"]},
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
    assert result.elapsed_seconds >= 0
    second_turn_messages = provider.requests[1].messages
    assert isinstance(second_turn_messages[-2], AssistantMessage)
    assert second_turn_messages[-2].tool_calls[0].call_id == "call-1"
    assert isinstance(second_turn_messages[-1], ToolResultMessage)
    assert second_turn_messages[-1].call_id == "call-1"
    assert [event.type for event in observed] == [
        "model.request.prepared",
        "tool.call.requested",
        "tool.call.completed",
        "model.request.prepared",
        "final.output",
    ]


@pytest.mark.asyncio
async def test_orchestrator_filters_provider_tools_by_agent_spec(tmp_path) -> None:
    async def echo(arguments: dict[str, object]) -> str:
        return str(arguments["text"])

    echo_spec = ToolSpec(
        name="echo",
        description="Echo",
        input_schema={
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": False,
        },
    )
    write_spec = ToolSpec(
        name="filesystem.write",
        description="Write",
        input_schema={"type": "object", "additionalProperties": False},
    )
    registry = InMemoryToolRegistry((echo_spec, write_spec))
    provider = ToolThenFinalProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"echo": echo}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    agent = default_agent().model_copy(update={"tools": ("echo",)})

    await orchestrator.run(AgentRunRequest(prompt="use a tool", agent=agent))

    assert {tool.name for tool in provider.requests[0].tools} == {"echo"}


@pytest.mark.asyncio
async def test_orchestrator_composes_skill_context_and_audits_metadata(tmp_path) -> None:
    skill_root = tmp_path / "skills"
    _write_skill(
        skill_root / "alpha",
        name="alpha",
        instructions="Secret skill body should reach the model only.",
    )
    registry = InMemoryToolRegistry(())
    provider = CapturingFinalProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        skill_composer=SkillComposer(
            SkillResolver((FilesystemSkillRepository(skill_root),))
        ),
    )
    agent = default_agent().model_copy(update={"skills": ("alpha",)})

    await orchestrator.run(
        AgentRunRequest(
            prompt="@skill:alpha help",
            agent=agent,
            active_skills=("alpha",),
        )
    )

    instructions = provider.requests[0].instructions
    assert "[Available skills]" in instructions
    assert "[Active skills]" in instructions
    assert "Secret skill body should reach the model only." in instructions
    audit_text = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
    assert "skills.selected" in audit_text
    assert "alpha" in audit_text
    assert "Secret skill body should reach the model only." not in audit_text


@pytest.mark.asyncio
async def test_orchestrator_emits_prepared_model_request_without_persisting_it(
    tmp_path,
) -> None:
    observed: list[RunEvent] = []
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    provider = CapturingFinalProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=InMemoryToolRegistry(()),
        tool_executor=FunctionToolExecutor({}, InMemoryToolRegistry(())),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        event_observer=observed.append,
    )

    await orchestrator.run(AgentRunRequest(prompt="hello", agent=default_agent()))

    prepared = [event for event in observed if isinstance(event, ModelRequestPreparedEvent)]
    assert len(prepared) == 1
    assert prepared[0].instructions == provider.requests[0].instructions
    assert prepared[0].messages == tuple(
        message.model_dump(mode="json") for message in provider.requests[0].messages
    )
    assert prepared[0].tools == tuple(
        tool.model_dump(mode="json") for tool in provider.requests[0].tools
    )
    persisted = await state.list_events("run-1")
    assert all(not isinstance(event, ModelRequestPreparedEvent) for event in persisted)


@pytest.mark.asyncio
async def test_orchestrator_trims_oldest_turns_to_request_byte_budget(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    provider = CapturingFinalProvider()
    registry = InMemoryToolRegistry(())
    await state.append_message("session-1", "run-0", UserMessage(content="x" * 5_000))
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        context_service=ContextService(
            state,
            JsonlAuditSink(tmp_path / "context-audit.jsonl"),
            config=ContextConfig(auto_compaction=False, max_request_bytes=1_500),
        ),
    )

    await orchestrator.run(
        AgentRunRequest(
            prompt="current prompt",
            agent=default_agent("model-a"),
            session_id="session-1",
        )
    )

    assert len(provider.requests) == 1
    assert provider.requests[0].messages == (UserMessage(content="current prompt"),)


@pytest.mark.asyncio
async def test_orchestrator_limits_default_agent_tool_schemas_to_request_budget(
    tmp_path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    huge_spec = ToolSpec(
        name="huge",
        description="x" * 10_000,
        input_schema={"type": "object", "properties": {}, "additionalProperties": False},
    )
    search_spec = ToolSpec(
        name="tool.search",
        description="Find relevant tools by keyword.",
        input_schema={
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": False,
        },
    )
    registry = InMemoryToolRegistry((huge_spec, search_spec))
    provider = CapturingFinalProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        context_service=ContextService(
            state,
            JsonlAuditSink(tmp_path / "context-audit.jsonl"),
            config=ContextConfig(auto_compaction=False, max_request_bytes=100_000),
        ),
    )

    await orchestrator.run(AgentRunRequest(prompt="hello", agent=default_agent("model-a")))

    assert len(provider.requests) == 1
    assert provider.requests[0].tools == (search_spec,)


@pytest.mark.asyncio
async def test_orchestrator_expands_tool_schemas_after_tool_search(tmp_path) -> None:
    async def tool_search_handler(_arguments: dict[str, object]) -> str:
        return json.dumps({"tools": [{"name": "echo", "description": "Echo text."}]})

    async def echo_handler(arguments: dict[str, object]) -> str:
        return str(arguments["text"])

    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    echo_spec = ToolSpec(
        name="echo",
        description="x" * 5_000,
        input_schema={
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": False,
        },
    )
    search_spec = ToolSpec(
        name="tool.search",
        description="Find relevant tools by keyword.",
        input_schema={
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": False,
        },
    )
    registry = InMemoryToolRegistry((echo_spec, search_spec))
    provider = ToolSearchThenEchoProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(
            {"tool.search": tool_search_handler, "echo": echo_handler},
            registry,
        ),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        context_service=ContextService(
            state,
            JsonlAuditSink(tmp_path / "context-audit.jsonl"),
            config=ContextConfig(auto_compaction=False, max_request_bytes=100_000),
        ),
    )

    result = await orchestrator.run(
        AgentRunRequest(prompt="hello", agent=default_agent("model-a"))
    )

    assert result.final_output == "done"
    assert tuple(tool.name for tool in provider.requests[0].tools) == ("tool.search",)
    assert tuple(tool.name for tool in provider.requests[1].tools) == (
        "tool.search",
        "echo",
    )


@pytest.mark.asyncio
async def test_orchestrator_fails_fast_when_explicit_tools_exceed_request_byte_budget(
    tmp_path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    spec = ToolSpec(
        name="huge",
        description="x" * 2_000,
        input_schema={"type": "object", "properties": {}, "additionalProperties": False},
    )
    registry = InMemoryToolRegistry((spec,))
    provider = CapturingFinalProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        context_service=ContextService(
            state,
            JsonlAuditSink(tmp_path / "context-audit.jsonl"),
            config=ContextConfig(auto_compaction=False, max_request_bytes=1_024),
        ),
    )

    with pytest.raises(ProviderError, match=r"fixed_bytes=.*tool_count=1"):
        await orchestrator.run(
            AgentRunRequest(
                prompt="hello",
                agent=default_agent("model-a").model_copy(update={"tools": ("huge",)}),
            )
        )

    assert provider.requests == []


@pytest.mark.asyncio
async def test_orchestrator_validates_active_skill_required_tools_before_provider(
    tmp_path,
) -> None:
    skill_root = tmp_path / "skills"
    _write_skill(skill_root / "alpha", name="alpha", required_tools=["echo"])
    registry = InMemoryToolRegistry(())
    provider = CapturingFinalProvider()
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        skill_composer=SkillComposer(
            SkillResolver((FilesystemSkillRepository(skill_root),))
        ),
    )
    agent = default_agent().model_copy(update={"skills": ("alpha",)})

    with pytest.raises(ColossusError, match="requires unavailable tools"):
        await orchestrator.run(
            AgentRunRequest(prompt="help", agent=agent, active_skills=("alpha",))
        )
    assert provider.requests == []


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
    assert [event.type for event in observed] == [
        "model.request.prepared",
        "reasoning.summary",
    ]


@pytest.mark.asyncio
async def test_orchestrator_rejects_empty_response_after_tool_turn(tmp_path) -> None:
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
    orchestrator = AgentOrchestrator(
        provider=TextToolThenEmptyProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"echo": echo}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    with pytest.raises(ProviderError, match="no assistant text or tool calls"):
        await orchestrator.run(AgentRunRequest(prompt="read", agent=default_agent()))


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
async def test_orchestrator_injects_active_skills_before_tool_validation(tmp_path) -> None:
    skill_root = tmp_path / "skills"
    _write_skill(skill_root / "alpha", name="alpha")
    observed_arguments: dict[str, object] = {}

    async def read_resource(arguments: dict[str, object]) -> str:
        observed_arguments.update(arguments)
        return json.dumps({"resource": {"path": arguments["path"], "content": "guide"}})

    spec = ToolSpec(
        name="skill.resource.read",
        description="Read skill resource",
        input_schema={
            "type": "object",
            "properties": {
                "skill": {"type": "string"},
                "path": {"type": "string"},
                "active_skills": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["skill", "path", "active_skills"],
            "additionalProperties": False,
        },
    )
    registry = InMemoryToolRegistry((spec,))
    orchestrator = AgentOrchestrator(
        provider=SkillResourceToolThenFinalProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({"skill.resource.read": read_resource}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        skill_composer=SkillComposer(SkillResolver((FilesystemSkillRepository(skill_root),))),
    )
    agent = default_agent().model_copy(update={"skills": ("alpha",)})

    result = await orchestrator.run(
        AgentRunRequest(prompt="read guide", agent=agent, active_skills=("alpha",))
    )

    assert result.final_output == "done"
    assert observed_arguments["active_skills"] == ["alpha"]


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
    assert decisions[0].title == "Run tests and lint"
    assert decisions[0].decision == "Run tests and lint."
    assert (
        decisions[0].intent
        == "The user wants this explicit instruction treated as a durable commitment."
    )
    assert (
        decisions[0].applies_when
        == "Future turns in this session when the commitment is relevant."
    )
    assert decisions[0].source_excerpt == "Mvoing forward I want to make sure run tests and lint"
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
async def test_orchestrator_does_not_auto_capture_non_standalone_key_decision_prompt(
    tmp_path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    decision_service = DecisionService(state, audit)
    provider = CapturingFinalProvider()
    registry = InMemoryToolRegistry(())
    orchestrator = AgentOrchestrator(
        provider=provider,
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
            prompt="Can you make sure the provider tests still pass?",
            agent=default_agent(),
            session_id="session-1",
        )
    )

    assert result.final_output == "done"
    assert await decision_service.list_decisions(session_id="session-1") == ()


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
        "model.request.prepared",
        "tool.call.requested",
        "tool.call.completed",
        "model.request.prepared",
        "final.output",
    ]


@pytest.mark.asyncio
async def test_orchestrator_recovers_from_malformed_provider_tool_arguments(
    tmp_path,
) -> None:
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
    provider = MalformedToolArgumentsThenFinalProvider()
    observed: list[RunEvent] = []
    state_store = SQLiteStateStore(tmp_path / "state.sqlite3")
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state_store,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        event_observer=observed.append,
    )

    result = await orchestrator.run(
        AgentRunRequest(prompt="use a tool", agent=default_agent(max_turns=3))
    )

    assert result.final_output == "recovered"
    assert result.events_recorded == 2
    assert len(provider.requests) == 2
    recovery = next(event for event in observed if isinstance(event, ErrorEvent))
    assert recovery.recoverable is True
    assert "attempt=1/2" in recovery.message
    assert [event.type for event in observed] == [
        "model.request.prepared",
        "error",
        "model.request.prepared",
        "final.output",
    ]
    persisted = await state_store.list_events("run-1")
    assert any(isinstance(event, ErrorEvent) and event.recoverable for event in persisted)
    audit_records = [
        json.loads(line)
        for line in (tmp_path / "audit.jsonl").read_text(encoding="utf-8").splitlines()
    ]
    assert [record["event"] for record in audit_records] == [
        "provider.tool_call_recovery",
        "run.completed",
    ]


@pytest.mark.asyncio
async def test_orchestrator_exhausts_malformed_tool_argument_retries(tmp_path) -> None:
    provider = AlwaysMalformedToolArgumentsProvider()
    registry = InMemoryToolRegistry(())
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=SQLiteStateStore(tmp_path / "state.sqlite3"),
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    with pytest.raises(ProviderError, match="invalid JSON for tool call arguments"):
        await orchestrator.run(
            AgentRunRequest(prompt="use a tool", agent=default_agent(max_turns=4))
        )

    assert len(provider.requests) == 3
    audit_records = [
        json.loads(line)
        for line in (tmp_path / "audit.jsonl").read_text(encoding="utf-8").splitlines()
    ]
    events = [record["event"] for record in audit_records]
    assert events.count("provider.tool_call_recovery") == 2
    assert events[-1] == "provider.tool_call_recovery_exhausted"


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


@pytest.mark.asyncio
async def test_orchestrator_does_not_persist_unmatched_tool_call_on_denial(tmp_path) -> None:
    state_store = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=ShellToolProvider(),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=DenyByDefaultApprovalHandler(),
        state_store=state_store,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
    )

    with pytest.raises(ColossusError, match="denied"):
        await orchestrator.run(
            AgentRunRequest(
                prompt="write a note",
                agent=default_agent(),
                session_id="session-1",
            )
        )

    messages = await state_store.list_messages("session-1")
    assert [message.role for message in messages] == ["user"]


def _write_skill(
    path,
    *,
    name: str,
    instructions: str = "instructions",
    required_tools: list[str] | None = None,
) -> None:
    path.mkdir(parents=True)
    (path / "manifest.json").write_text(
        json.dumps(
            {
                "name": name,
                "version": "1.0.0",
                "description": f"{name} skill",
                "triggers": [name],
                "required_tools": required_tools or [],
                "permissions": [],
                "offline_compatible": True,
            }
        ),
        encoding="utf-8",
    )
    (path / "SKILL.md").write_text(instructions, encoding="utf-8")
