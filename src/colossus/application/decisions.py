"""Application service for durable key decisions."""

from uuid import uuid4

from colossus.domain.decisions import (
    DecisionPriority,
    DecisionSource,
    DecisionStatus,
    KeyDecision,
    utc_now_iso,
)
from colossus.domain.errors import ColossusError
from colossus.ports.audit import AuditSink
from colossus.ports.state import StateStore


class DecisionService:
    def __init__(self, state_store: StateStore, audit_sink: AuditSink) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink

    async def create_decision(
        self,
        *,
        session_id: str,
        title: str,
        decision: str,
        source: DecisionSource = "agent",
        priority: DecisionPriority = "normal",
        intent: str = "",
        applies_when: str = "",
        rationale: str = "",
        source_excerpt: str = "",
        goal_id: str | None = None,
        plan_id: str | None = None,
        supersedes: str | None = None,
        decision_id: str | None = None,
    ) -> KeyDecision:
        if not title:
            raise ColossusError("Decision title is required.")
        if not decision:
            raise ColossusError("Decision text is required.")
        resolved_id = decision_id or f"kd_{uuid4().hex[:12]}"
        existing = await self._state_store.get_decision(resolved_id)
        if existing is not None:
            raise ColossusError(f"Decision already exists: {resolved_id}")
        await self._state_store.ensure_session(session_id, title=title[:80])
        now = utc_now_iso()
        key_decision = KeyDecision(
            id=resolved_id,
            session_id=session_id,
            goal_id=goal_id,
            plan_id=plan_id,
            source=source,
            status="active",
            priority=priority,
            title=title,
            decision=decision,
            intent=intent,
            applies_when=applies_when,
            rationale=rationale,
            source_excerpt=source_excerpt,
            supersedes=supersedes,
            created_at=now,
            updated_at=now,
        )
        await self._state_store.save_decision(key_decision)
        await self._audit_sink.record(
            source,
            "decision.created",
            {
                "decision_id": key_decision.id,
                "session_id": key_decision.session_id,
                "priority": key_decision.priority,
                "source": key_decision.source,
            },
        )
        return key_decision

    async def update_decision(
        self,
        decision_id: str,
        *,
        session_id: str | None = None,
        title: str | None = None,
        decision: str | None = None,
        priority: DecisionPriority | None = None,
        intent: str | None = None,
        applies_when: str | None = None,
        rationale: str | None = None,
        source_excerpt: str | None = None,
        status: DecisionStatus | None = None,
        goal_id: str | None = None,
        plan_id: str | None = None,
    ) -> KeyDecision:
        key_decision = await self._require_decision(decision_id)
        if session_id is not None and key_decision.session_id != session_id:
            raise ColossusError(f"Decision {decision_id} does not belong to session {session_id}.")
        changes: dict[str, object] = {"updated_at": utc_now_iso()}
        if title is not None:
            if not title:
                raise ColossusError("Decision title cannot be empty.")
            changes["title"] = title
        if decision is not None:
            if not decision:
                raise ColossusError("Decision text cannot be empty.")
            changes["decision"] = decision
        if priority is not None:
            changes["priority"] = priority
        if intent is not None:
            changes["intent"] = intent
        if applies_when is not None:
            changes["applies_when"] = applies_when
        if rationale is not None:
            changes["rationale"] = rationale
        if source_excerpt is not None:
            changes["source_excerpt"] = source_excerpt
        if status is not None:
            changes["status"] = status
        if goal_id is not None:
            changes["goal_id"] = goal_id
        if plan_id is not None:
            changes["plan_id"] = plan_id
        updated = key_decision.model_copy(update=changes)
        await self._state_store.save_decision(updated)
        await self._audit_sink.record(
            "agent",
            "decision.updated",
            {
                "decision_id": updated.id,
                "session_id": updated.session_id,
                "status": updated.status,
                "priority": updated.priority,
            },
        )
        return updated

    async def archive_decision(
        self,
        decision_id: str,
        *,
        session_id: str | None = None,
    ) -> KeyDecision:
        archived = await self.update_decision(
            decision_id,
            session_id=session_id,
            status="archived",
        )
        await self._audit_sink.record(
            "agent",
            "decision.archived",
            {"decision_id": archived.id, "session_id": archived.session_id},
        )
        return archived

    async def supersede_decision(
        self,
        decision_id: str,
        *,
        session_id: str | None = None,
        title: str,
        decision: str,
        source: DecisionSource = "agent",
        priority: DecisionPriority = "normal",
        intent: str = "",
        applies_when: str = "",
        rationale: str = "",
        source_excerpt: str = "",
        goal_id: str | None = None,
        plan_id: str | None = None,
    ) -> KeyDecision:
        old = await self._require_decision(decision_id)
        if session_id is not None and old.session_id != session_id:
            raise ColossusError(f"Decision {decision_id} does not belong to session {session_id}.")
        await self.update_decision(old.id, status="superseded")
        replacement = await self.create_decision(
            session_id=old.session_id,
            title=title,
            decision=decision,
            source=source,
            priority=priority,
            intent=intent,
            applies_when=applies_when,
            rationale=rationale,
            source_excerpt=source_excerpt,
            goal_id=goal_id or old.goal_id,
            plan_id=plan_id or old.plan_id,
            supersedes=old.id,
        )
        await self._audit_sink.record(
            source,
            "decision.superseded",
            {
                "decision_id": old.id,
                "replacement_id": replacement.id,
                "session_id": old.session_id,
            },
        )
        return replacement

    async def get_decision(self, decision_id: str) -> KeyDecision:
        return await self._require_decision(decision_id)

    async def list_decisions(
        self,
        *,
        session_id: str | None = None,
        status: DecisionStatus | None = "active",
    ) -> tuple[KeyDecision, ...]:
        return await self._state_store.list_decisions(session_id=session_id, status=status)

    async def _require_decision(self, decision_id: str) -> KeyDecision:
        decision = await self._state_store.get_decision(decision_id)
        if decision is None:
            raise ColossusError(f"Decision not found: {decision_id}")
        return decision
