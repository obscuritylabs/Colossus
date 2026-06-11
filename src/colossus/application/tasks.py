"""Session task application service."""

from datetime import UTC, datetime
from uuid import uuid4

from colossus.domain.errors import ColossusError
from colossus.domain.tasks import Task, TaskStatus
from colossus.ports.audit import AuditSink
from colossus.ports.state import StateStore


class TaskService:
    def __init__(self, state_store: StateStore, audit_sink: AuditSink) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink

    async def create_task(
        self,
        *,
        session_id: str,
        title: str,
        description: str = "",
        status: TaskStatus = "pending",
        task_id: str | None = None,
    ) -> Task:
        if not title:
            raise ColossusError("Task title is required.")
        resolved_id = task_id or f"task-{uuid4().hex[:12]}"
        existing = await self._state_store.get_task(resolved_id)
        if existing is not None:
            raise ColossusError(f"Task already exists: {resolved_id}")
        await self._state_store.ensure_session(session_id, title=title[:80])
        now = _now()
        task = Task(
            id=resolved_id,
            session_id=session_id,
            title=title,
            description=description,
            status=status,
            created_at=now,
            updated_at=now,
        )
        await self._state_store.save_task(task)
        await self._audit_sink.record(
            "agent",
            "task.created",
            {"task_id": task.id, "session_id": session_id, "status": task.status},
        )
        return task

    async def update_task(
        self,
        task_id: str,
        *,
        session_id: str | None = None,
        title: str | None = None,
        description: str | None = None,
        status: TaskStatus | None = None,
    ) -> Task:
        task = await self._require_task(task_id)
        if session_id is not None and task.session_id != session_id:
            raise ColossusError(f"Task {task_id} does not belong to session {session_id}.")
        changes: dict[str, object] = {"updated_at": _now()}
        if title is not None:
            if not title:
                raise ColossusError("Task title cannot be empty.")
            changes["title"] = title
        if description is not None:
            changes["description"] = description
        if status is not None:
            changes["status"] = status
        updated = task.model_copy(update=changes)
        await self._state_store.save_task(updated)
        await self._audit_sink.record(
            "agent",
            "task.updated",
            {"task_id": updated.id, "session_id": updated.session_id, "status": updated.status},
        )
        return updated

    async def get_task(self, task_id: str) -> Task:
        return await self._require_task(task_id)

    async def list_tasks(
        self,
        *,
        session_id: str | None = None,
        status: TaskStatus | None = None,
    ) -> tuple[Task, ...]:
        return await self._state_store.list_tasks(session_id=session_id, status=status)

    async def _require_task(self, task_id: str) -> Task:
        task = await self._state_store.get_task(task_id)
        if task is None:
            raise ColossusError(f"Task not found: {task_id}")
        return task


def _now() -> str:
    return datetime.now(tz=UTC).isoformat()
