"""Provider-neutral agent orchestration loop."""

import json
import re
from collections.abc import Callable
from time import perf_counter
from uuid import uuid4

from colossus.application.context import ContextService
from colossus.application.decisions import DecisionService
from colossus.application.risk import RiskAssessmentService
from colossus.application.skills import SkillComposer
from colossus.application.subagents import SubagentService
from colossus.application.tools import validate_tool_call
from colossus.domain.context import ContextBuildResult
from colossus.domain.errors import PolicyDeniedError, ProviderError, ToolExecutionError
from colossus.domain.events import (
    ApprovalAutoGrantedEvent,
    ApprovalRequestedEvent,
    ContextPreparedEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ModelRequestPreparedEvent,
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
SESSION_CONTEXT_TOOLS = frozenset(
    {
        "task.create",
        "task.update",
        "task.list",
        "decision.create",
        "decision.update",
        "decision.list",
        "decision.archive",
        "decision.supersede",
        "agent.delegate",
        "agent.result",
        "agent.list",
    }
)
RUN_CONTEXT_TOOLS = frozenset({"agent.delegate"})
ACTIVE_SKILL_CONTEXT_TOOLS = frozenset({"skill.resource.list", "skill.resource.read"})
GOAL_CONTEXT_TOOLS = frozenset({"goal.show", "goal.update"})
TOOL_SCHEMA_BUDGET_PRIORITY = (
    "tool.search",
    "filesystem.search",
    "filesystem.read",
    "filesystem.list",
    "git.status",
    "git.diff",
    "shell.run",
    "web.fetch",
    "docs.fetch",
    "mcp.tools",
    "skill.resource.list",
    "skill.resource.read",
    "context.show",
    "memory.search",
    "task.list",
    "decision.list",
    "goal.show",
    "user.ask",
)


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
        subagent_service: SubagentService | None = None,
        decision_service: DecisionService | None = None,
        skill_composer: SkillComposer | None = None,
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
        self._subagent_service = subagent_service
        self._decision_service = decision_service
        self._skill_composer = skill_composer
        if subagent_service is not None and event_observer is not None:
            subagent_service.set_event_observer(event_observer)

    def set_event_observer(self, event_observer: RunEventObserver | None) -> None:
        self._event_observer = event_observer
        if self._subagent_service is not None:
            self._subagent_service.set_event_observer(event_observer)

    async def run(self, request: AgentRunRequest) -> AgentRunResult:
        started = perf_counter()
        run_id = self._run_id_factory()
        all_tool_specs = self._tool_specs_for_agent(request.agent.tools)
        if request.goal_id is None:
            all_tool_specs = tuple(
                spec for spec in all_tool_specs if spec.name not in GOAL_CONTEXT_TOOLS
            )
        tool_specs = all_tool_specs
        dynamic_tool_expansion = not request.agent.tools
        if (
            dynamic_tool_expansion
            and self._context_service is not None
            and self._context_service.config.max_request_bytes is not None
        ):
            tool_specs = _enforce_tool_schema_budget(
                tool_specs,
                max_request_bytes=self._context_service.config.max_request_bytes,
                tool_schema_budget_percent=(
                    self._context_service.config.tool_schema_budget_percent
                ),
            )
        instructions = request.agent.instructions
        skill_context = None
        if self._skill_composer is not None:
            skill_context = self._skill_composer.compose(
                instructions=request.agent.instructions,
                agent=request.agent,
                prompt=request.prompt,
                active_skills=request.active_skills,
                skill_mode_enabled=request.skill_mode_enabled,
                tools=tool_specs,
            )
            instructions = skill_context.instructions
        active_skill_names = (
            tuple(skill.manifest.name for skill in skill_context.active_skills)
            if skill_context is not None
            else ()
        )
        messages: list[Message] = []
        if request.session_id is not None:
            await self._state_store.ensure_session(request.session_id, title=request.prompt[:80])
            messages.extend(await self._state_store.list_messages(request.session_id))
        if skill_context is not None:
            await self._audit_sink.record(
                "agent",
                "skills.selected",
                {
                    "run_id": run_id,
                    "skill_mode_enabled": request.skill_mode_enabled,
                    "available_skill_count": len(skill_context.available_skills),
                    "active_skills": skill_context.active_metadata,
                },
            )
        user_message = UserMessage(content=request.prompt)
        messages.append(user_message)
        if request.session_id is not None:
            await self._state_store.append_message(request.session_id, run_id, user_message)
            captured = await self._capture_user_key_decision(request.prompt, request.session_id)
            if captured is not None and _is_standalone_key_decision_prompt(request.prompt):
                final_text = f"Noted as key decision {captured}."
                assistant_message = AssistantMessage(content=final_text)
                await self._state_store.append_message(
                    request.session_id,
                    run_id,
                    assistant_message,
                )
                await self._audit_sink.record(
                    "agent",
                    "run.completed",
                    {
                        "run_id": run_id,
                        "events": 0,
                        "key_decision_id": captured,
                        "elapsed_ms": _elapsed_ms(started),
                    },
                )
                return AgentRunResult(
                    run_id=run_id,
                    final_output=final_text,
                    events_recorded=0,
                    session_id=request.session_id,
                    elapsed_seconds=_elapsed_seconds(started),
                )
        final_text = ""
        events_recorded = 0

        for turn in range(request.agent.max_turns):
            prepared_messages = tuple(messages)
            if self._context_service is not None:
                context_result = await self._context_service.prepare_messages(
                    session_id=request.session_id,
                    model=request.agent.model,
                    instructions=instructions,
                    messages=prepared_messages,
                    provider=self._context_provider or self._provider,
                    summary_model=self._context_model,
                )
                prepared_messages = context_result.messages
                self._observe_event(
                    _context_prepared_event(turn, request.agent.model, context_result)
                )
            model_request = ModelRequest(
                model=request.agent.model,
                instructions=instructions,
                messages=prepared_messages,
                tools=tool_specs,
            )
            request_byte_estimate = _model_request_byte_estimate(model_request)
            if (
                self._context_service is not None
                and self._context_service.config.max_request_bytes is not None
            ):
                model_request, request_byte_estimate = _enforce_model_request_byte_budget(
                    model_request,
                    self._context_service.config.max_request_bytes,
                )
            self._observe_event(
                _model_request_prepared_event(
                    turn,
                    model_request,
                    request_byte_estimate=request_byte_estimate,
                )
            )
            pending_tool_calls: list[ToolCall] = []
            collected_text: list[str] = []
            turn_final_text = ""

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
                    turn_final_text = event.text

            if not pending_tool_calls and not _has_visible_text(collected_text, turn_final_text):
                await self._audit_sink.record(
                    "agent",
                    "provider.empty_response",
                    {"run_id": run_id, "events": events_recorded},
                )
                raise ProviderError(
                    "Provider returned no assistant text or tool calls. "
                    "For OpenAI-compatible endpoints, verify that streaming chat chunks "
                    "include choices[].delta.content or choices[].delta.tool_calls."
                )

            turn_assistant_message: AssistantMessage | None = None
            if collected_text or pending_tool_calls:
                turn_assistant_message = AssistantMessage(
                    content="".join(collected_text),
                    tool_calls=tuple(pending_tool_calls),
                )
                messages.append(turn_assistant_message)

            if not pending_tool_calls:
                final_text = turn_final_text or "".join(collected_text)
                if (
                    turn_assistant_message is not None
                    and request.session_id is not None
                ):
                    await self._state_store.append_message(
                        request.session_id,
                        run_id,
                        turn_assistant_message,
                    )
                await self._audit_sink.record(
                    "agent",
                    "run.completed",
                    {
                        "run_id": run_id,
                        "events": events_recorded,
                        "elapsed_ms": _elapsed_ms(started),
                    },
                )
                return AgentRunResult(
                    run_id=run_id,
                    final_output=final_text,
                    events_recorded=events_recorded,
                    session_id=request.session_id,
                    elapsed_seconds=_elapsed_seconds(started),
                )

            tool_messages: list[ToolResultMessage] = []
            for call in pending_tool_calls:
                result = await self._execute_tool(
                    run_id,
                    call,
                    request.session_id,
                    request.goal_id,
                    active_skill_names,
                )
                if dynamic_tool_expansion and call.name == "tool.search":
                    tool_specs = _expand_tool_specs_from_tool_search(
                        tool_specs,
                        all_tool_specs,
                        result.output,
                    )
                tool_message = ToolResultMessage(
                    call_id=result.call_id,
                    name=result.name,
                    content=result.output,
                )
                tool_messages.append(tool_message)
                messages.append(tool_message)
                events_recorded += 1
            if request.session_id is not None and turn_assistant_message is not None:
                await self._state_store.append_message(
                    request.session_id,
                    run_id,
                    turn_assistant_message,
                )
                for tool_message in tool_messages:
                    await self._state_store.append_message(
                        request.session_id,
                        run_id,
                        tool_message,
                    )

        await self._audit_sink.record(
            "agent",
            "run.max_turns",
            {
                "run_id": run_id,
                "events": events_recorded,
                "elapsed_ms": _elapsed_ms(started),
            },
        )
        return AgentRunResult(
            run_id=run_id,
            final_output=final_text,
            events_recorded=events_recorded,
            session_id=request.session_id,
            elapsed_seconds=_elapsed_seconds(started),
        )

    def tool_specs(self) -> tuple[ToolSpec, ...]:
        return self._tool_registry.list_specs()

    def _tool_specs_for_agent(self, allowed_tools: tuple[str, ...]) -> tuple[ToolSpec, ...]:
        specs = self._tool_registry.list_specs()
        if not allowed_tools:
            return specs
        allowed = set(allowed_tools)
        return tuple(spec for spec in specs if spec.name in allowed)

    async def _execute_tool(
        self,
        run_id: str,
        call: ToolCall,
        session_id: str | None = None,
        goal_id: str | None = None,
        active_skill_names: tuple[str, ...] = (),
    ) -> ToolCallCompletedEvent:
        spec = self._tool_registry.get_spec(call.name)
        if spec is None:
            return await self._record_tool_error(
                run_id,
                call,
                category="unknown_tool",
                message=f"Unknown tool requested: {call.name}",
                audit_action="tool.unknown",
            )
        call = _with_execution_context(call, session_id, run_id, goal_id, active_skill_names)
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
                    if self._risk_auto_approve:
                        decision = decision.model_copy(
                            update={
                                "decision": "requires_approval",
                                "reason": f"Risk assessment denied {call.name}: {risk.summary}",
                            }
                        )
                        await self._audit_sink.record(
                            "agent",
                            "risk.requires_approval",
                            {"run_id": run_id, "tool": call.name, "summary": risk.summary},
                        )
                    else:
                        await self._audit_sink.record(
                            "agent",
                            "risk.denied",
                            {"run_id": run_id, "tool": call.name, "summary": risk.summary},
                        )
                        raise PolicyDeniedError(
                            f"Risk assessment denied {call.name}: {risk.summary}"
                        )
                elif (
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

    async def _capture_user_key_decision(self, prompt: str, session_id: str) -> str | None:
        if self._decision_service is None or not _looks_like_key_decision_prompt(prompt):
            return None
        text = prompt.strip()
        active = await self._decision_service.list_decisions(session_id=session_id)
        for decision in active:
            if decision.decision == text:
                return decision.id
        decision = await self._decision_service.create_decision(
            session_id=session_id,
            title=_decision_title(text),
            decision=text,
            source="user",
            priority="high",
            rationale="Captured from a durable user preference before provider execution.",
        )
        return decision.id


def _with_execution_context(
    call: ToolCall,
    session_id: str | None,
    run_id: str,
    goal_id: str | None = None,
    active_skill_names: tuple[str, ...] = (),
) -> ToolCall:
    arguments = dict(call.arguments)
    changed = False
    if (
        session_id is not None
        and call.name in SESSION_CONTEXT_TOOLS
        and "session_id" not in arguments
    ):
        arguments["session_id"] = session_id
        changed = True
    if call.name in RUN_CONTEXT_TOOLS:
        if "parent_run_id" not in arguments:
            arguments["parent_run_id"] = run_id
            changed = True
        if "parent_call_id" not in arguments:
            arguments["parent_call_id"] = call.call_id
            changed = True
    if call.name in ACTIVE_SKILL_CONTEXT_TOOLS:
        arguments["active_skills"] = list(active_skill_names)
        changed = True
    if goal_id is not None and call.name in GOAL_CONTEXT_TOOLS and "goal_id" not in arguments:
        arguments["goal_id"] = goal_id
        changed = True
    if not changed:
        return call
    return call.model_copy(update={"arguments": arguments})


def _model_request_prepared_event(
    turn: int,
    request: ModelRequest,
    *,
    request_byte_estimate: int,
) -> ModelRequestPreparedEvent:
    return ModelRequestPreparedEvent(
        turn=turn,
        model=request.model,
        instructions=request.instructions,
        messages=tuple(message.model_dump(mode="json") for message in request.messages),
        tools=tuple(tool.model_dump(mode="json") for tool in request.tools),
        request_byte_estimate=request_byte_estimate,
    )


def _model_request_byte_estimate(request: ModelRequest) -> int:
    payload = {
        "model": request.model,
        "instructions": request.instructions,
        "messages": tuple(message.model_dump(mode="json") for message in request.messages),
        "tools": tuple(tool.model_dump(mode="json") for tool in request.tools),
    }
    return len(json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def _enforce_tool_schema_budget(
    tools: tuple[ToolSpec, ...],
    *,
    max_request_bytes: int,
    tool_schema_budget_percent: float | None,
) -> tuple[ToolSpec, ...]:
    if not tools or tool_schema_budget_percent is None:
        return tools
    max_tool_schema_bytes = int(max_request_bytes * tool_schema_budget_percent)
    if max_tool_schema_bytes <= 0:
        return ()
    if _tool_schema_byte_estimate(tools) <= max_tool_schema_bytes:
        return tools

    selected: list[ToolSpec] = []
    for spec in _prioritized_tool_specs(tools):
        candidate = (*selected, spec)
        if _tool_schema_byte_estimate(candidate) <= max_tool_schema_bytes:
            selected.append(spec)
    return tuple(selected)


def _tool_schema_byte_estimate(tools: tuple[ToolSpec, ...]) -> int:
    payload = tuple(tool.model_dump(mode="json") for tool in tools)
    return len(json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def _prioritized_tool_specs(tools: tuple[ToolSpec, ...]) -> tuple[ToolSpec, ...]:
    tools_by_name = {tool.name: tool for tool in tools}
    prioritized: list[ToolSpec] = []
    seen: set[str] = set()
    for name in TOOL_SCHEMA_BUDGET_PRIORITY:
        spec = tools_by_name.get(name)
        if spec is None:
            continue
        prioritized.append(spec)
        seen.add(name)
    prioritized.extend(tool for tool in tools if tool.name not in seen)
    return tuple(prioritized)


def _expand_tool_specs_from_tool_search(
    current_tools: tuple[ToolSpec, ...],
    available_tools: tuple[ToolSpec, ...],
    output: str,
) -> tuple[ToolSpec, ...]:
    requested_names = _tool_search_result_names(output)
    if not requested_names:
        return current_tools
    current_names = {tool.name for tool in current_tools}
    available_by_name = {tool.name: tool for tool in available_tools}
    expanded = list(current_tools)
    for name in requested_names:
        if name in current_names:
            continue
        spec = available_by_name.get(name)
        if spec is None:
            continue
        expanded.append(spec)
        current_names.add(name)
    return tuple(expanded)


def _tool_search_result_names(output: str) -> tuple[str, ...]:
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return ()
    if not isinstance(payload, dict):
        return ()
    tools = payload.get("tools")
    if not isinstance(tools, list):
        return ()
    names: list[str] = []
    seen: set[str] = set()
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        name = tool.get("name")
        if not isinstance(name, str) or not name or name in seen:
            continue
        names.append(name)
        seen.add(name)
    return tuple(names)


def _enforce_model_request_byte_budget(
    request: ModelRequest,
    max_request_bytes: int,
) -> tuple[ModelRequest, int]:
    request_bytes = _model_request_byte_estimate(request)
    if request_bytes <= max_request_bytes:
        return request, request_bytes

    fixed_request = request.model_copy(update={"messages": ()})
    fixed_bytes = _model_request_byte_estimate(fixed_request)
    if fixed_bytes > max_request_bytes:
        raise ProviderError(
            "Prepared model request exceeds the configured request body byte limit before "
            "message history is included. "
            f"request_bytes={request_bytes} fixed_bytes={fixed_bytes} "
            f"max_request_bytes={max_request_bytes} tool_count={len(request.tools)}. "
            "Reduce the tool catalog for this agent or raise context.max_request_bytes."
        )

    messages = list(request.messages)
    while len(messages) > 1 and request_bytes > max_request_bytes:
        trim_to = _next_user_message_index(messages)
        if trim_to is None:
            break
        messages = messages[trim_to:]
        trimmed_request = request.model_copy(update={"messages": tuple(messages)})
        request_bytes = _model_request_byte_estimate(trimmed_request)
        request = trimmed_request

    if request_bytes > max_request_bytes:
        raise ProviderError(
            "Prepared model request exceeds the configured request body byte limit after "
            "older conversation turns were trimmed. "
            f"request_bytes={request_bytes} max_request_bytes={max_request_bytes} "
            f"message_count={len(request.messages)} tool_count={len(request.tools)}. "
            "The latest user turn, instructions, or tool schemas are too large for this limit."
        )
    return request, request_bytes


def _next_user_message_index(messages: list[Message]) -> int | None:
    for index, message in enumerate(messages[1:], start=1):
        if isinstance(message, UserMessage):
            return index
    return None


def _context_prepared_event(
    turn: int,
    model: str,
    result: ContextBuildResult,
) -> ContextPreparedEvent:
    return ContextPreparedEvent(
        turn=turn,
        model=model,
        token_estimate=result.token_estimate,
        original_token_estimate=result.original_token_estimate,
        context_window_tokens=result.context_window_tokens,
        threshold_tokens=result.threshold_tokens,
        target_tokens=result.target_tokens,
        snapshot_id=result.snapshot_id,
        compacted=result.compacted,
        snapshot_created=result.snapshot_created,
    )


def _elapsed_seconds(started: float) -> float:
    return max(perf_counter() - started, 0.0)


def _elapsed_ms(started: float) -> int:
    return round(_elapsed_seconds(started) * 1000)


_KEY_DECISION_PROMPT_PATTERN = re.compile(
    r"\b(moving forward|mvoing forward|going forward|from now on|remember this|"
    r"please remember|make sure)\b",
    re.IGNORECASE,
)


def _looks_like_key_decision_prompt(prompt: str) -> bool:
    return bool(_KEY_DECISION_PROMPT_PATTERN.search(prompt.strip()))


def _is_standalone_key_decision_prompt(prompt: str) -> bool:
    text = prompt.strip()
    if not _looks_like_key_decision_prompt(text):
        return False
    if "?" in text:
        return False
    return len(text) <= 220


def _has_visible_text(chunks: list[str], final_text: str) -> bool:
    return bool(final_text.strip() or "".join(chunks).strip())


def _decision_title(prompt: str) -> str:
    title = prompt.strip().replace("\n", " ")
    return title[:80] or "Key decision"
