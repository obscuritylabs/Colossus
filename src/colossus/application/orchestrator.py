"""Provider-neutral agent orchestration loop."""

import json
from collections.abc import Callable
from uuid import uuid4

from colossus.application.context import ContextService
from colossus.application.risk import RiskAssessmentService
from colossus.application.tools import validate_tool_call
from colossus.domain.errors import PolicyDeniedError, ToolExecutionError
from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    RunEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.messages import AssistantMessage, Message, ToolResultMessage, UserMessage
from colossus.domain.requests import AgentRunRequest, AgentRunResult, ModelRequest
from colossus.domain.tools import ToolCall, ToolSpec
from colossus.ports.approval import ApprovalHandler
from colossus.ports.audit import AuditSink
from colossus.ports.model_provider import ModelProvider
from colossus.ports.policy import PolicyEngine
from colossus.ports.state import StateStore
from colossus.ports.tools import ToolExecutor, ToolRegistry

RunIdFactory = Callable[[], str]
RunEventObserver = Callable[[RunEvent], None]
SESSION_CONTEXT_TOOLS = frozenset({"task.create", "task.update", "task.list"})


class AgentOrchestrator:
    def __init__(
        self,
        *,
        provider: ModelProvider,
        tool_registry: ToolRegistry,
        tool_executor: ToolExecutor,
        policy_engine: PolicyEngine,
        approval_handler: ApprovalHandler,
        state_store: StateStore,
        audit_sink: AuditSink,
        run_id_factory: RunIdFactory | None = None,
        context_service: ContextService | None = None,
        context_provider: ModelProvider | None = None,
        context_model: str | None = None,
        event_observer: RunEventObserver | None = None,
        risk_assessment_service: RiskAssessmentService | None = None,
        risk_auto_approve: bool = False,
        auto_approve_required_tools: bool = False,
    ) -> None:
        self._provider = provider
        self._tool_registry = tool_registry
        self._tool_executor = tool_executor
        self._policy_engine = policy_engine
        self._approval_handler = approval_handler
        self._state_store = state_store
        self._audit_sink = audit_sink
        self._run_id_factory = run_id_factory or (lambda: str(uuid4()))
        self._context_service = context_service
        self._context_provider = context_provider
        self._context_model = context_model
        self._event_observer = event_observer
        self._risk_assessment_service = risk_assessment_service
        self._risk_auto_approve = risk_auto_approve
        self._auto_approve_required_tools = auto_approve_required_tools

    def set_event_observer(self, event_observer: RunEventObserver | None) -> None:
        self._event_observer = event_observer

    async def run(self, request: AgentRunRequest) -> AgentRunResult:
        run_id = self._run_id_factory()
        messages: list[Message] = []
        if request.session_id is not None:
            await self._state_store.ensure_session(request.session_id, title=request.prompt[:80])
            messages.extend(await self._state_store.list_messages(request.session_id))
        user_message = UserMessage(content=request.prompt)
        messages.append(user_message)
        if request.session_id is not None:
            await self._state_store.append_message(request.session_id, run_id, user_message)
        final_text = ""
        events_recorded = 0

        for _turn in range(request.agent.max_turns):
            prepared_messages = tuple(messages)
            if self._context_service is not None:
                context_result = await self._context_service.prepare_messages(
                    session_id=request.session_id,
                    model=request.agent.model,
                    instructions=request.agent.instructions,
                    messages=prepared_messages,
                    provider=self._context_provider or self._provider,
                    summary_model=self._context_model,
                )
                prepared_messages = context_result.messages
            model_request = ModelRequest(
                model=request.agent.model,
                instructions=request.agent.instructions,
                messages=prepared_messages,
                tools=self._tool_registry.list_specs(),
            )
            pending_tool_calls: list[ToolCall] = []
            collected_text: list[str] = []

            async for event in self._provider.stream(model_request):
                events_recorded += 1
                await self._state_store.append_event(run_id, event)
                self._observe_event(event)
                if isinstance(event, ModelDeltaEvent):
                    collected_text.append(event.text)
                elif isinstance(event, ToolCallRequestedEvent):
                    pending_tool_calls.append(
                        ToolCall(
                            call_id=event.call_id,
                            name=event.name,
                            arguments=event.arguments,
                        )
                    )
                elif isinstance(event, FinalOutputEvent):
                    final_text = event.text

            if collected_text or pending_tool_calls:
                assistant_message = AssistantMessage(
                    content="".join(collected_text),
                    tool_calls=tuple(pending_tool_calls),
                )
                messages.append(assistant_message)
                if request.session_id is not None:
                    await self._state_store.append_message(
                        request.session_id,
                        run_id,
                        assistant_message,
                    )

            if not pending_tool_calls:
                if not final_text:
                    final_text = "".join(collected_text)
                await self._audit_sink.record(
                    "agent",
                    "run.completed",
                    {"run_id": run_id, "events": events_recorded},
                )
                return AgentRunResult(
                    run_id=run_id,
                    final_output=final_text,
                    events_recorded=events_recorded,
                    session_id=request.session_id,
                )

            for call in pending_tool_calls:
                result = await self._execute_tool(run_id, call, request.session_id)
                tool_message = ToolResultMessage(
                    call_id=result.call_id,
                    name=result.name,
                    content=result.output,
                )
                messages.append(tool_message)
                if request.session_id is not None:
                    await self._state_store.append_message(request.session_id, run_id, tool_message)
                events_recorded += 1

        await self._audit_sink.record(
            "agent",
            "run.max_turns",
            {"run_id": run_id, "events": events_recorded},
        )
        return AgentRunResult(
            run_id=run_id,
            final_output=final_text,
            events_recorded=events_recorded,
            session_id=request.session_id,
        )

    def tool_specs(self) -> tuple[ToolSpec, ...]:
        return self._tool_registry.list_specs()

    async def _execute_tool(
        self,
        run_id: str,
        call: ToolCall,
        session_id: str | None = None,
    ) -> ToolCallCompletedEvent:
        call = _with_session_context(call, session_id)
        spec = self._tool_registry.get_spec(call.name)
        if spec is None:
            return await self._record_tool_error(
                run_id,
                call,
                category="unknown_tool",
                message=f"Unknown tool requested: {call.name}",
                audit_action="tool.unknown",
            )
        try:
            validate_tool_call(spec, call)
        except ToolExecutionError as exc:
            return await self._record_tool_error(
                run_id,
                call,
                category="invalid_arguments",
                message=str(exc),
                audit_action="tool.invalid",
            )
        decision = self._policy_engine.decide_tool_call(spec, call)
        await self._audit_sink.record(
            "agent",
            "tool.policy",
            {"run_id": run_id, "tool": call.name, "decision": decision.decision},
        )
        if decision.decision == "deny":
            raise PolicyDeniedError(decision.reason)
        risk = None
        if self._risk_assessment_service is not None and call.name == "shell.run":
            risk = await self._risk_assessment_service.assess_tool_call(spec, call, decision)
            if risk is not None:
                risk_event = risk.to_event(call.call_id)
                await self._state_store.append_event(run_id, risk_event)
                self._observe_event(risk_event)
                await self._audit_sink.record(
                    "agent",
                    "risk.assessed",
                    {
                        "run_id": run_id,
                        "tool": call.name,
                        "risk_level": risk.risk_level,
                        "recommended_decision": risk.recommended_decision,
                        "model_role": risk.model_role,
                        "profile": risk.profile_name,
                        "summary": risk.summary,
                    },
                )
                if risk.recommended_decision == "deny":
                    await self._audit_sink.record(
                        "agent",
                        "risk.denied",
                        {"run_id": run_id, "tool": call.name, "summary": risk.summary},
                    )
                    raise PolicyDeniedError(f"Risk assessment denied {call.name}: {risk.summary}")
                if (
                    decision.decision == "allow"
                    and risk.recommended_decision == "requires_approval"
                ):
                    decision = decision.model_copy(
                        update={
                            "decision": "requires_approval",
                            "reason": f"Risk assessment requires approval: {risk.summary}",
                        }
                    )
                elif decision.decision == "requires_approval":
                    decision = decision.model_copy(
                        update={
                            "reason": (
                                f"{decision.reason} "
                                f"Risk assessment {risk.risk_level}: {risk.summary}"
                            )
                        }
                    )
            else:
                await self._audit_sink.record(
                    "agent",
                    "risk.review_unavailable",
                    {"run_id": run_id, "tool": call.name},
                )
        if (
            decision.decision == "requires_approval"
            and risk is not None
            and self._risk_auto_approve
            and risk.risk_level == "low"
            and risk.recommended_decision == "allow"
        ):
            reason = f"Risk assessment auto-approved low-risk {call.name}: {risk.summary}"
            auto_event = ApprovalAutoGrantedEvent(call_id=call.call_id, reason=reason)
            await self._state_store.append_event(run_id, auto_event)
            self._observe_event(auto_event)
            await self._audit_sink.record(
                "agent",
                "risk.auto_approved",
                {"run_id": run_id, "tool": call.name, "summary": risk.summary},
            )
        elif decision.decision == "requires_approval" and self._auto_approve_required_tools:
            reason = f"Full-access auto-approved {call.name}: {decision.reason}"
            auto_event = ApprovalAutoGrantedEvent(call_id=call.call_id, reason=reason)
            await self._state_store.append_event(run_id, auto_event)
            self._observe_event(auto_event)
            await self._audit_sink.record(
                "agent",
                "tool.auto_approved",
                {
                    "run_id": run_id,
                    "tool": call.name,
                    "mode": "full-access",
                    "reason": decision.reason,
                },
            )
        elif decision.decision == "requires_approval":
            approval_event = ApprovalRequestedEvent(call_id=call.call_id, reason=decision.reason)
            await self._state_store.append_event(run_id, approval_event)
            self._observe_event(approval_event)
            approved = await self._approval_handler.approve(call, decision)
            if not approved:
                await self._audit_sink.record(
                    "agent",
                    "tool.denied",
                    {"run_id": run_id, "tool": call.name, "reason": decision.reason},
                )
                raise PolicyDeniedError(f"Tool call denied by approval handler: {call.name}")
        try:
            result = await self._tool_executor.execute(call)
        except ToolExecutionError as exc:
            return await self._record_tool_error(
                run_id,
                call,
                category="execution_error",
                message=str(exc),
                audit_action="tool.error",
            )
        completed = ToolCallCompletedEvent(
            call_id=result.call_id,
            name=result.name,
            output=result.output,
            exit_code=result.exit_code,
        )
        await self._state_store.append_event(run_id, completed)
        self._observe_event(completed)
        await self._audit_sink.record(
            "tool",
            "tool.completed",
            {"run_id": run_id, "tool": call.name, "exit_code": result.exit_code},
        )
        return completed

    async def _record_tool_error(
        self,
        run_id: str,
        call: ToolCall,
        *,
        category: str,
        message: str,
        audit_action: str,
    ) -> ToolCallCompletedEvent:
        output = json.dumps(
            {
                "error": {
                    "type": category,
                    "message": message,
                    "tool": call.name,
                    "recoverable": True,
                }
            },
            sort_keys=True,
        )
        completed = ToolCallCompletedEvent(
            call_id=call.call_id,
            name=call.name,
            output=output,
            exit_code=1,
        )
        await self._state_store.append_event(run_id, completed)
        self._observe_event(completed)
        await self._audit_sink.record(
            "tool",
            audit_action,
            {"run_id": run_id, "tool": call.name, "reason": message},
        )
        return completed

    def _observe_event(self, event: RunEvent) -> None:
        if self._event_observer is not None:
            self._event_observer(event)


def _with_session_context(call: ToolCall, session_id: str | None) -> ToolCall:
    if session_id is None or call.name not in SESSION_CONTEXT_TOOLS:
        return call
    if "session_id" in call.arguments:
        return call
    return call.model_copy(update={"arguments": {**call.arguments, "session_id": session_id}})
