"""Plan-mode application service."""

from uuid import uuid4

from colossus.domain.errors import ColossusError
from colossus.domain.plans import Plan, PlanStep, utc_now_iso
from colossus.ports.audit import AuditSink
from colossus.ports.state import StateStore


class PlanService:
    def __init__(self, state_store: StateStore, audit_sink: AuditSink) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink

    async def create_plan(self, prompt: str, session_id: str, content: str = "") -> Plan:
        await self._state_store.ensure_session(session_id, title=prompt[:80])
        plan = Plan(
            id=str(uuid4()),
            session_id=session_id,
            prompt=prompt,
            content=content,
            steps=(
                PlanStep(
                    index=1,
                    title="Clarify Objective",
                    detail=(
                        "Restate the requested outcome and identify constraints from repo context."
                    ),
                ),
                PlanStep(
                    index=2,
                    title="Inspect Current State",
                    detail=(
                        "Read relevant files, tests, configuration, docs, and existing behavior."
                    ),
                ),
                PlanStep(
                    index=3,
                    title="Implement Scoped Changes",
                    detail="Make the minimum cohesive code and documentation changes required.",
                    requires_mutation=True,
                ),
                PlanStep(
                    index=4,
                    title="Verify",
                    detail="Run focused tests, static checks, and smoke commands.",
                ),
                PlanStep(
                    index=5,
                    title="Report Outcome",
                    detail="Summarize changed behavior, verification, and remaining risks.",
                ),
            ),
        )
        await self._state_store.save_plan(plan)
        await self._audit_sink.record(
            "agent",
            "plan.created",
            {
                "plan_id": plan.id,
                "session_id": session_id,
                "requires_approval": plan.requires_approval,
            },
        )
        return plan

    async def replace_draft_plan(self, plan_id: str, prompt: str, content: str) -> Plan:
        plan = await self._require_plan(plan_id)
        if plan.status != "draft":
            raise ColossusError(f"Plan {plan_id} is not a draft.")
        updated = plan.model_copy(
            update={
                "prompt": prompt,
                "content": content,
                "updated_at": utc_now_iso(),
            }
        )
        await self._state_store.save_plan(updated)
        await self._audit_sink.record(
            "agent",
            "plan.updated",
            {"plan_id": plan_id, "session_id": updated.session_id},
        )
        return updated

    async def approve_plan(self, plan_id: str) -> Plan:
        plan = await self._require_plan(plan_id)
        approved = plan.model_copy(update={"status": "approved", "updated_at": utc_now_iso()})
        await self._state_store.save_plan(approved)
        await self._audit_sink.record("user", "plan.approved", {"plan_id": plan_id})
        return approved

    async def mark_executed(self, plan_id: str, run_id: str) -> Plan:
        plan = await self._require_plan(plan_id)
        executed = plan.model_copy(update={"status": "executed", "updated_at": utc_now_iso()})
        await self._state_store.save_plan(executed)
        await self._audit_sink.record(
            "agent",
            "plan.executed",
            {"plan_id": plan_id, "run_id": run_id},
        )
        return executed

    async def get_plan(self, plan_id: str) -> Plan:
        return await self._require_plan(plan_id)

    async def list_plans(self, session_id: str | None = None) -> tuple[Plan, ...]:
        return await self._state_store.list_plans(session_id)

    async def require_approved(self, plan_id: str) -> Plan:
        plan = await self._require_plan(plan_id)
        if plan.status != "approved":
            raise ColossusError(f"Plan {plan_id} is not approved.")
        return plan

    async def _require_plan(self, plan_id: str) -> Plan:
        plan = await self._state_store.get_plan(plan_id)
        if plan is None:
            raise ColossusError(f"Plan not found: {plan_id}")
        return plan
