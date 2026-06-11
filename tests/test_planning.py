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
    assert plan.content == ""
    assert plan.created_at
    assert plan.updated_at


@pytest.mark.asyncio
async def test_plan_service_persists_and_replaces_markdown_content(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = PlanService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))

    plan = await service.create_plan("# First\n\nPlan body", "session-1", content="# First")
    updated = await service.replace_draft_plan(plan.id, "ship the thing", "# Updated")

    assert updated.id == plan.id
    assert updated.prompt == "ship the thing"
    assert updated.content == "# Updated"
    assert updated.status == "draft"
    assert (await service.get_plan(plan.id)).content == "# Updated"


def test_plan_model_accepts_legacy_payload_without_content() -> None:
    from colossus.domain.plans import Plan

    plan = Plan.model_validate({"id": "plan-1", "session_id": "session-1", "prompt": "ship"})

    assert plan.content == ""
    assert plan.status == "draft"
    assert plan.created_at


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
