"""Goal-mode application services."""

from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from time import perf_counter
from uuid import uuid4

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.agents import AgentSpec
from colossus.domain.errors import ColossusError
from colossus.domain.goals import Goal, GoalStatus
from colossus.domain.plans import Plan
from colossus.domain.requests import AgentRunRequest, AgentRunResult
from colossus.ports.audit import AuditSink
from colossus.ports.state import StateStore

GoalTurnRunner = Callable[[AgentRunRequest], Awaitable[AgentRunResult]]

_GOAL_MODE_PROMPT = """\
You are running in Colossus goal mode.

Active goal id: {goal_id}
Objective: {objective}

Work autonomously in bounded, useful steps. Use the normal tools available to you.
When the objective is genuinely finished, call goal.update with status "complete"
and a concise summary. If you cannot make meaningful progress without user input or
an external state change, call goal.update with status "blocked" and explain why.
If work remains, leave the goal active and state the next useful step.
"""


class GoalTurnResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    iteration: int
    run_id: str
    final_output: str
    events_recorded: int
    elapsed_seconds: float = 0.0


class GoalRunResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    goal: Goal
    turns: tuple[GoalTurnResult, ...] = Field(default_factory=tuple)
    iteration_budget_exhausted: bool = False
    elapsed_seconds: float = 0.0


class GoalService:
    def __init__(self, state_store: StateStore, audit_sink: AuditSink) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink

    async def create_goal(
        self,
        *,
        objective: str,
        session_id: str,
        iteration_budget: int | None = None,
        source_plan_id: str | None = None,
        goal_id: str | None = None,
    ) -> Goal:
        normalized_objective = objective.strip()
        if not normalized_objective:
            raise ColossusError("Goal objective is required.")
        if iteration_budget is not None and iteration_budget < 1:
            raise ColossusError("Goal iteration budget must be at least 1.")
        resolved_id = goal_id or f"goal-{uuid4().hex[:12]}"
        existing = await self._state_store.get_goal(resolved_id)
        if existing is not None:
            raise ColossusError(f"Goal already exists: {resolved_id}")
        await self._state_store.ensure_session(session_id, title=normalized_objective[:80])
        now = _now()
        goal = Goal(
            id=resolved_id,
            session_id=session_id,
            objective=normalized_objective,
            source_plan_id=source_plan_id,
            iteration_budget=iteration_budget,
            created_at=now,
            updated_at=now,
        )
        await self._state_store.save_goal(goal)
        await self._audit_sink.record(
            "agent",
            "goal.created",
            {
                "goal_id": goal.id,
                "session_id": session_id,
                "iteration_budget": iteration_budget,
                "source_plan_id": source_plan_id,
            },
        )
        return goal

    async def get_goal(self, goal_id: str) -> Goal:
        goal = await self._state_store.get_goal(goal_id)
        if goal is None:
            raise ColossusError(f"Goal not found: {goal_id}")
        return goal

    async def list_goals(
        self,
        *,
        session_id: str | None = None,
        status: GoalStatus | None = None,
    ) -> tuple[Goal, ...]:
        return await self._state_store.list_goals(session_id=session_id, status=status)

    async def update_goal(
        self,
        goal_id: str,
        *,
        session_id: str | None = None,
        status: GoalStatus | None = None,
        summary: str | None = None,
        blocked_reason: str | None = None,
        iterations_completed: int | None = None,
    ) -> Goal:
        goal = await self.get_goal(goal_id)
        if session_id is not None and goal.session_id != session_id:
            raise ColossusError(f"Goal {goal_id} does not belong to session {session_id}.")
        changes: dict[str, object] = {"updated_at": _now()}
        if status is not None:
            changes["status"] = status
            if status != "blocked" and blocked_reason is None:
                changes["blocked_reason"] = ""
        if summary is not None:
            changes["summary"] = summary
        if blocked_reason is not None:
            changes["blocked_reason"] = blocked_reason
        if iterations_completed is not None:
            changes["iterations_completed"] = max(iterations_completed, 0)
        updated = goal.model_copy(update=changes)
        await self._state_store.save_goal(updated)
        await self._audit_sink.record(
            "agent",
            "goal.updated",
            {
                "goal_id": updated.id,
                "session_id": updated.session_id,
                "status": updated.status,
                "iterations_completed": updated.iterations_completed,
            },
        )
        return updated

    async def record_iteration(self, goal_id: str) -> Goal:
        goal = await self.get_goal(goal_id)
        return await self.update_goal(
            goal_id,
            iterations_completed=goal.iterations_completed + 1,
        )


class GoalLoopService:
    def __init__(
        self,
        goal_service: GoalService,
        turn_runner: GoalTurnRunner,
        audit_sink: AuditSink,
    ) -> None:
        self._goal_service = goal_service
        self._turn_runner = turn_runner
        self._audit_sink = audit_sink

    async def run(
        self,
        *,
        objective: str,
        agent: AgentSpec,
        session_id: str,
        max_iterations: int,
        source_plan_id: str | None = None,
        active_skills: tuple[str, ...] = (),
        skill_mode_enabled: bool = True,
    ) -> GoalRunResult:
        if max_iterations < 1:
            raise ColossusError("Goal mode requires at least one iteration.")
        started = perf_counter()
        goal = await self._goal_service.create_goal(
            objective=objective,
            session_id=session_id,
            iteration_budget=max_iterations,
            source_plan_id=source_plan_id,
        )
        await self._audit_sink.record(
            "agent",
            "goal.loop.started",
            {
                "goal_id": goal.id,
                "session_id": session_id,
                "max_iterations": max_iterations,
                "source_plan_id": source_plan_id,
            },
        )
        turns: list[GoalTurnResult] = []
        for iteration in range(1, max_iterations + 1):
            current = await self._goal_service.get_goal(goal.id)
            if current.status != "active":
                break
            request = AgentRunRequest(
                prompt=_iteration_prompt(current, iteration),
                agent=_goal_agent(agent, current),
                session_id=session_id,
                goal_id=current.id,
                plan_id=current.source_plan_id,
                skill_mode_enabled=skill_mode_enabled,
                active_skills=active_skills,
            )
            result = await self._turn_runner(request)
            turns.append(
                GoalTurnResult(
                    iteration=iteration,
                    run_id=result.run_id,
                    final_output=result.final_output,
                    events_recorded=result.events_recorded,
                    elapsed_seconds=result.elapsed_seconds,
                )
            )
            await self._goal_service.record_iteration(goal.id)
        final_goal = await self._goal_service.get_goal(goal.id)
        exhausted = final_goal.status == "active" and len(turns) >= max_iterations
        if exhausted:
            await self._audit_sink.record(
                "agent",
                "goal.loop.iteration_budget_exhausted",
                {
                    "goal_id": final_goal.id,
                    "session_id": session_id,
                    "iterations_completed": final_goal.iterations_completed,
                },
            )
        await self._audit_sink.record(
            "agent",
            "goal.loop.finished",
            {
                "goal_id": final_goal.id,
                "session_id": session_id,
                "status": final_goal.status,
                "iterations_completed": final_goal.iterations_completed,
                "elapsed_ms": _elapsed_ms(started),
            },
        )
        return GoalRunResult(
            goal=final_goal,
            turns=tuple(turns),
            iteration_budget_exhausted=exhausted,
            elapsed_seconds=_elapsed_seconds(started),
        )


def _goal_agent(agent: AgentSpec, goal: Goal) -> AgentSpec:
    prompt = _GOAL_MODE_PROMPT.format(goal_id=goal.id, objective=goal.objective)
    instructions = f"{agent.instructions.rstrip()}\n\n{prompt}"
    return agent.model_copy(update={"instructions": instructions})


def goal_objective_from_plan(plan: Plan) -> str:
    parts = [
        f"Execute approved plan {plan.id}.",
        "",
        "Original request:",
        plan.prompt.strip(),
    ]
    content = plan.content.strip()
    if content:
        parts.extend(("", "Approved plan:", content))
    parts.extend(
        (
            "",
            "Execution contract:",
            "- Treat the approved plan as the starting contract for the goal loop.",
            "- Follow the plan in bounded, verifiable steps.",
            "- Revise the plan only when new evidence makes a step unsafe, impossible, or stale.",
            "- Use goal.update to mark the goal complete when the approved work is finished.",
            "- Use goal.update to mark the goal blocked if user input or external state "
            "is required.",
        )
    )
    return "\n".join(parts)


def _iteration_prompt(goal: Goal, iteration: int) -> str:
    if iteration == 1:
        return f"Start goal mode for {goal.id}: {goal.objective}"
    return (
        f"Continue goal mode for {goal.id}.\n"
        f"Objective: {goal.objective}\n"
        "Use the session history for prior work. If the objective is complete or blocked, "
        "call goal.update with the appropriate status."
    )


def _now() -> str:
    return datetime.now(tz=UTC).isoformat()


def _elapsed_seconds(started: float) -> float:
    return max(perf_counter() - started, 0.0)


def _elapsed_ms(started: float) -> int:
    return round(_elapsed_seconds(started) * 1000)
