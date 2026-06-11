import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.planning import PlanService
from colossus.domain.errors import ColossusError


@pytest.mark.asyncio
async def test_plan_service_creates_and_approves_plan(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = PlanService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))

    plan = await service.create_plan("ship readiness", "session-1")
    approved = await service.approve_plan(plan.id)

    assert plan.status == "draft"
    assert plan.requires_approval is True
    assert approved.status == "approved"
    assert (await service.get_plan(plan.id)).status == "approved"


@pytest.mark.asyncio
async def test_plan_service_rejects_unapproved_execution(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = PlanService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    plan = await service.create_plan("ship readiness", "session-1")

    with pytest.raises(ColossusError):
        await service.require_approved(plan.id)


@pytest.mark.asyncio
async def test_plan_service_marks_executed(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = PlanService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    plan = await service.create_plan("ship readiness", "session-1")
    await service.approve_plan(plan.id)

    executed = await service.mark_executed(plan.id, "run-1")

    assert executed.status == "executed"
