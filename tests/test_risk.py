from collections.abc import AsyncIterator

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.approvals import AllowAllApprovalHandler, DenyByDefaultApprovalHandler
from colossus.application.defaults import default_agent
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.risk import RiskAssessmentService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.errors import PolicyDeniedError
from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    FinalOutputEvent,
    RiskAssessmentEvent,
    RunEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.models import ResolvedModelProfile
from colossus.domain.policy import PolicyDecision
from colossus.domain.requests import AgentRunRequest, ModelRequest
from colossus.domain.tools import ToolCall, ToolPermission, ToolSpec


class RiskJsonProvider:
    name = "risk-json"

    def __init__(self, text: str) -> None:
        self._text = text
        self.captured_request: ModelRequest | None = None

    def capabilities(self):
        return ()

    async def check_readiness(self):
        raise NotImplementedError

    async def list_models(self):
        return ()

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.captured_request = request
        yield FinalOutputEvent(text=self._text)


class ShellToolProvider:
    name = "single-shell-tool"

    def __init__(self, arguments: dict[str, object]) -> None:
        self._arguments = arguments

    def capabilities(self):
        return ()

    async def check_readiness(self):
        raise NotImplementedError

    async def list_models(self):
        return ()

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if has_tool_result:
            yield FinalOutputEvent(text="done")
            return
        yield ToolCallRequestedEvent(
            call_id="call-1",
            name="shell.run",
            arguments=self._arguments,
        )


def _risk_router(provider: RiskJsonProvider) -> ModelRouter:
    profile = ResolvedModelProfile(
        role="risk_evaluator",
        profile_name="risk",
        provider="echo",
        model="risk-model",
    )
    return ModelRouter(
        {
            "risk_evaluator": ModelRoute(
                role="risk_evaluator",
                profile_name="risk",
                provider=provider,
                profile=profile,
            )
        }
    )


@pytest.mark.asyncio
async def test_risk_assessment_parses_structured_json_and_redacts_secrets() -> None:
    provider = RiskJsonProvider(
        '{"risk_level":"high","summary":"Deletes files",'
        '"concerns":["destructive"],"recommended_decision":"deny"}'
    )
    service = RiskAssessmentService(_risk_router(provider))

    result = await service.assess_tool_call(
        ToolSpec(
            name="shell.run",
            description="Run command.",
            input_schema={"type": "object"},
            permissions=ToolPermission(approval_required=True),
        ),
        ToolCall(
            call_id="call-1",
            name="shell.run",
            arguments={
                "argv": ["deploy", "--token=secret-value"],
                "env": {"API_KEY": "secret-value"},
            },
        ),
        PolicyDecision(decision="requires_approval", reason="Tool requires approval."),
    )

    assert result is not None
    assert result.risk_level == "high"
    assert result.recommended_decision == "deny"
    assert provider.captured_request is not None
    prompt = provider.captured_request.messages[0].content
    assert "secret-value" not in prompt
    assert "[REDACTED]" in prompt


@pytest.mark.asyncio
async def test_risk_assessment_invalid_json_is_unavailable() -> None:
    service = RiskAssessmentService(_risk_router(RiskJsonProvider("not json")))

    result = await service.assess_tool_call(
        ToolSpec(name="shell.run", description="Run command.", input_schema={"type": "object"}),
        ToolCall(call_id="call-1", name="shell.run", arguments={"argv": ["echo", "ok"]}),
        PolicyDecision(decision="requires_approval", reason="Tool requires approval."),
    )

    assert result is None


@pytest.mark.asyncio
async def test_shell_run_risk_deny_stops_before_execution_and_audits(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    risk_service = RiskAssessmentService(
        _risk_router(
            RiskJsonProvider(
                '{"risk_level":"high","summary":"Dangerous command",'
                '"concerns":[],"recommended_decision":"deny"}'
            )
        )
    )
    orchestrator = AgentOrchestrator(
        provider=ShellToolProvider({"argv": ["echo", "should-not-run"]}),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        risk_assessment_service=risk_service,
    )

    with pytest.raises(PolicyDeniedError, match="Risk assessment denied"):
        await orchestrator.run(AgentRunRequest(prompt="run", agent=default_agent()))

    events = await state.list_events("run-1")
    audit = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
    assert any(isinstance(event, RiskAssessmentEvent) for event in events)
    assert "risk.denied" in audit


@pytest.mark.asyncio
async def test_shell_run_risk_unavailable_falls_back_to_policy(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=ShellToolProvider({"argv": ["echo", "ok"]}),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        risk_assessment_service=RiskAssessmentService(_risk_router(RiskJsonProvider("nope"))),
    )

    result = await orchestrator.run(AgentRunRequest(prompt="run", agent=default_agent()))
    audit = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")

    assert result.final_output == "done"
    assert "risk.review_unavailable" in audit


@pytest.mark.asyncio
async def test_shell_run_low_risk_auto_approval_skips_prompt(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=ShellToolProvider({"argv": ["echo", "ok"]}),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=DenyByDefaultApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        risk_assessment_service=RiskAssessmentService(
            _risk_router(
                RiskJsonProvider(
                    '{"risk_level":"low","summary":"Benign echo",'
                    '"concerns":[],"recommended_decision":"allow"}'
                )
            )
        ),
        risk_auto_approve=True,
    )

    result = await orchestrator.run(AgentRunRequest(prompt="run", agent=default_agent()))
    events = await state.list_events("run-1")
    audit = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")

    assert result.final_output == "done"
    assert any(isinstance(event, ApprovalAutoGrantedEvent) for event in events)
    assert not any(isinstance(event, ApprovalRequestedEvent) for event in events)
    assert "risk.auto_approved" in audit


@pytest.mark.asyncio
async def test_shell_run_medium_risk_auto_mode_still_prompts(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    specs, handlers = create_builtin_tools(Workspace(tmp_path))
    registry = InMemoryToolRegistry(specs)
    orchestrator = AgentOrchestrator(
        provider=ShellToolProvider({"argv": ["echo", "ok"]}),
        tool_registry=registry,
        tool_executor=FunctionToolExecutor(handlers, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=DenyByDefaultApprovalHandler(),
        state_store=state,
        audit_sink=JsonlAuditSink(tmp_path / "audit.jsonl"),
        run_id_factory=lambda: "run-1",
        risk_assessment_service=RiskAssessmentService(
            _risk_router(
                RiskJsonProvider(
                    '{"risk_level":"medium","summary":"Needs human review",'
                    '"concerns":[],"recommended_decision":"allow"}'
                )
            )
        ),
        risk_auto_approve=True,
    )

    with pytest.raises(PolicyDeniedError, match="Tool call denied by approval handler"):
        await orchestrator.run(AgentRunRequest(prompt="run", agent=default_agent()))

    events = await state.list_events("run-1")
    assert any(isinstance(event, ApprovalRequestedEvent) for event in events)
