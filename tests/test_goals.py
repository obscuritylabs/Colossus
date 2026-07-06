from collections.abc import AsyncIterator

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.defaults import default_agent
from colossus.application.goals import GoalLoopService, GoalService, goal_objective_from_plan
from colossus.domain.events import FinalOutputEvent, RunEvent, ToolCallRequestedEvent
from colossus.domain.messages import ToolResultMessage
from colossus.domain.plans import Plan
from colossus.domain.requests import AgentRunRequest, AgentRunResult, ModelRequest
from colossus.infrastructure.container import create_default_orchestrator


class GoalUpdateThenFinalProvider:
    name = "goal-update-then-final"

    def __init__(self) -> None:
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        if any(isinstance(message, ToolResultMessage) for message in request.messages):
            yield FinalOutputEvent(text="done")
            return
        yield ToolCallRequestedEvent(
            call_id="call-1",
            name="goal.update",
            arguments={"status": "complete", "summary": "Finished the goal."},
        )


@pytest.mark.asyncio
async def test_goal_service_persists_goal_status(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    service = GoalService(state, audit)

    created = await service.create_goal(
        objective="Ship the feature",
        session_id="session-1",
        iteration_budget=3,
        source_plan_id="plan-1",
        goal_id="goal-1",
    )
    updated = await service.update_goal(
        created.id,
        status="complete",
        summary="Feature shipped.",
    )
    listed = await service.list_goals(session_id="session-1", status="complete")

    assert updated.status == "complete"
    assert updated.summary == "Feature shipped."
    assert updated.source_plan_id == "plan-1"
    assert listed == (updated,)


@pytest.mark.asyncio
async def test_goal_loop_stops_when_goal_is_marked_complete(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    goal_service = GoalService(state, audit)
    captured: list[AgentRunRequest] = []

    async def runner(request: AgentRunRequest) -> AgentRunResult:
        captured.append(request)
        assert request.goal_id is not None
        await goal_service.update_goal(
            request.goal_id,
            status="complete",
            summary="Done.",
        )
        return AgentRunResult(
            run_id="run-1",
            final_output="done",
            events_recorded=1,
            elapsed_seconds=0.25,
        )

    loop = GoalLoopService(goal_service, runner, audit)

    result = await loop.run(
        objective="Finish the task",
        agent=default_agent(),
        session_id="session-1",
        max_iterations=5,
        source_plan_id="plan-1",
    )

    assert result.goal.status == "complete"
    assert result.goal.iterations_completed == 1
    assert result.goal.source_plan_id == "plan-1"
    assert len(result.turns) == 1
    assert result.turns[0].elapsed_seconds == 0.25
    assert result.elapsed_seconds >= 0
    assert captured[0].goal_id == result.goal.id
    assert captured[0].plan_id == "plan-1"
    assert "goal.update" in captured[0].agent.instructions


@pytest.mark.asyncio
async def test_goal_loop_reports_iteration_budget_exhaustion(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    goal_service = GoalService(state, audit)

    async def runner(request: AgentRunRequest) -> AgentRunResult:
        assert request.goal_id is not None
        return AgentRunResult(
            run_id=request.goal_id,
            final_output="still working",
            events_recorded=1,
        )

    loop = GoalLoopService(goal_service, runner, audit)

    result = await loop.run(
        objective="Keep working",
        agent=default_agent(),
        session_id="session-1",
        max_iterations=2,
    )

    assert result.goal.status == "active"
    assert result.goal.iterations_completed == 2
    assert result.iteration_budget_exhausted is True


@pytest.mark.asyncio
async def test_goal_tools_are_exposed_only_for_goal_runs_and_update_goal(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    goal_service = GoalService(state, audit)
    provider = GoalUpdateThenFinalProvider()
    orchestrator = create_default_orchestrator(
        tmp_path,
        provider,
        workspace_root=tmp_path,
        state_store=state,
        audit_sink=audit,
        goal_service=goal_service,
    )

    await orchestrator.run(AgentRunRequest(prompt="plain run", agent=default_agent()))

    plain_tool_names = {tool.name for tool in provider.requests[-1].tools}
    assert "goal.update" not in plain_tool_names
    goal = await goal_service.create_goal(objective="Finish", session_id="session-1")

    await orchestrator.run(
        AgentRunRequest(
            prompt="goal run",
            agent=default_agent(),
            session_id="session-1",
            goal_id=goal.id,
        )
    )

    goal_tool_names = {tool.name for tool in provider.requests[-2].tools}
    updated = await goal_service.get_goal(goal.id)
    assert "goal.show" in goal_tool_names
    assert "goal.update" in goal_tool_names
    assert updated.status == "complete"


def test_goal_objective_from_plan_includes_plan_contract() -> None:
    plan = Plan(
        id="plan-1",
        session_id="session-1",
        prompt="Ship the feature",
        content="# Plan\n\n- Implement\n- Test",
    )

    objective = goal_objective_from_plan(plan)

    assert "Execute approved plan plan-1." in objective
    assert "Original request:\nShip the feature" in objective
    assert "Approved plan:\n# Plan" in objective
    assert "Use goal.update" in objective
