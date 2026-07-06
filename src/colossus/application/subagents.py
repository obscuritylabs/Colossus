"""Durable queued subagent execution service."""

import asyncio
from collections.abc import Awaitable, Callable
from uuid import uuid4

from colossus.domain.errors import ColossusError
from colossus.domain.events import SubagentStatusEvent
from colossus.domain.requests import AgentRunResult
from colossus.domain.subagents import (
    SubagentJob,
    SubagentQueueStatus,
    SubagentStatus,
    utc_now_iso,
)
from colossus.ports.audit import AuditSink
from colossus.ports.state import StateStore

SubagentRunner = Callable[[SubagentJob], Awaitable[AgentRunResult]]
SubagentEventObserver = Callable[[SubagentStatusEvent], None]


class SubagentService:
    def __init__(
        self,
        state_store: StateStore,
        audit_sink: AuditSink,
        *,
        max_concurrent: int = 4,
    ) -> None:
        if max_concurrent < 1:
            raise ColossusError("subagents.max_concurrent must be at least 1.")
        self._state_store = state_store
        self._audit_sink = audit_sink
        self._max_concurrent = max_concurrent
        self._runner: SubagentRunner | None = None
        self._event_observer: SubagentEventObserver | None = None
        self._running: dict[str, asyncio.Task[None]] = {}
        self._started = False
        self._schedule_lock = asyncio.Lock()

    @property
    def max_concurrent(self) -> int:
        return self._max_concurrent

    def set_runner(self, runner: SubagentRunner) -> None:
        self._runner = runner

    def set_event_observer(self, observer: SubagentEventObserver | None) -> None:
        self._event_observer = observer

    async def start(self) -> None:
        if self._started:
            await self._schedule_queued()
            return
        self._started = True
        await self.mark_stale_running_interrupted()
        await self._schedule_queued()

    async def mark_stale_running_interrupted(self) -> None:
        for job in await self._state_store.list_subagent_jobs(status="running"):
            await self._save(
                job,
                status="interrupted",
                completed_at=utc_now_iso(),
                error="Subagent process exited before the job completed.",
            )
            await self._audit_sink.record(
                "agent",
                "subagent.interrupted",
                {"job_id": job.id, "session_id": job.session_id},
            )

    async def create_job(
        self,
        *,
        session_id: str,
        parent_run_id: str,
        parent_call_id: str,
        task: str,
        role: str = "subagent_default",
        job_id: str | None = None,
    ) -> SubagentJob:
        if not task.strip():
            raise ColossusError("Subagent task is required.")
        resolved_id = job_id or f"agent-{uuid4().hex[:12]}"
        existing = await self._state_store.get_subagent_job(resolved_id)
        if existing is not None:
            raise ColossusError(f"Subagent job already exists: {resolved_id}")
        job = SubagentJob(
            id=resolved_id,
            session_id=session_id,
            parent_run_id=parent_run_id,
            parent_call_id=parent_call_id,
            task=task,
            role=role or "subagent_default",
            child_session_id=f"{session_id}:subagent:{resolved_id}",
        )
        await self._state_store.save_subagent_job(job)
        await self._emit_status(
            job,
            "queued",
            "Subagent job queued.",
        )
        await self._audit_sink.record(
            "agent",
            "subagent.queued",
            {
                "job_id": job.id,
                "session_id": session_id,
                "parent_run_id": parent_run_id,
                "role": job.role,
            },
        )
        await self.start()
        return job

    async def get_job(self, job_id: str) -> SubagentJob:
        job = await self._state_store.get_subagent_job(job_id)
        if job is None:
            raise ColossusError(f"Subagent job not found: {job_id}")
        return job

    async def list_jobs(
        self,
        *,
        session_id: str | None = None,
        status: SubagentStatus | None = None,
    ) -> tuple[SubagentJob, ...]:
        await self.start()
        return await self._state_store.list_subagent_jobs(session_id=session_id, status=status)

    async def queue_status(self, *, session_id: str | None = None) -> SubagentQueueStatus:
        await self.start()
        jobs = await self._state_store.list_subagent_jobs(session_id=session_id)
        counts: dict[SubagentStatus, int] = dict.fromkeys(_subagent_statuses(), 0)
        for job in jobs:
            counts[job.status] += 1
        return SubagentQueueStatus(
            total=len(jobs),
            queued=counts["queued"],
            running=counts["running"],
            completed=counts["completed"],
            failed=counts["failed"],
            cancelled=counts["cancelled"],
            interrupted=counts["interrupted"],
            max_concurrent=self._max_concurrent,
            available_slots=max(self._max_concurrent - len(self._running), 0),
            runner_configured=self._runner is not None,
            started=self._started,
        )

    async def cancel_job(self, job_id: str) -> SubagentJob:
        job = await self.get_job(job_id)
        if job.status in {"completed", "failed", "cancelled", "interrupted"}:
            return job
        task = self._running.get(job_id)
        if task is not None:
            task.cancel()
        cancelled = await self._save(
            job,
            status="cancelled",
            completed_at=utc_now_iso(),
            error="Subagent job was cancelled.",
        )
        await self._emit_status(cancelled, "cancelled", "Subagent job cancelled.")
        await self._audit_sink.record(
            "agent",
            "subagent.cancelled",
            {"job_id": cancelled.id, "session_id": cancelled.session_id},
        )
        await self._schedule_queued()
        return cancelled

    async def resume_job(self, job_id: str) -> SubagentJob:
        job = await self.get_job(job_id)
        if job.status in {"queued", "running"}:
            return job
        if job.status == "completed":
            raise ColossusError(f"Completed subagent job cannot be resumed: {job_id}")
        resumed = await self._save(
            job,
            status="queued",
            child_run_id=None,
            final_output="",
            error="",
            started_at=None,
            completed_at=None,
        )
        await self._emit_status(resumed, "queued", "Subagent job resumed.")
        await self._audit_sink.record(
            "agent",
            "subagent.resumed",
            {"job_id": resumed.id, "session_id": resumed.session_id},
        )
        await self.start()
        return resumed

    async def drain(self, timeout_seconds: float | None = None) -> SubagentQueueStatus:
        await self.start()
        if timeout_seconds is not None and timeout_seconds < 0:
            raise ColossusError("Subagent drain timeout must be non-negative.")
        loop = asyncio.get_running_loop()
        deadline = None if timeout_seconds is None else loop.time() + timeout_seconds
        while self._running:
            tasks = tuple(self._running.values())
            if deadline is None:
                await asyncio.gather(*tasks, return_exceptions=True)
                continue
            remaining = deadline - loop.time()
            if remaining <= 0:
                break
            done, _pending = await asyncio.wait(
                tasks,
                timeout=remaining,
                return_when=asyncio.FIRST_COMPLETED,
            )
            if not done:
                break
        return await self.queue_status()

    async def _schedule_queued(self) -> None:
        if self._runner is None:
            return
        async with self._schedule_lock:
            queued = await self._state_store.list_subagent_jobs(status="queued")
            available = self._max_concurrent - len(self._running)
            for job in queued[: max(available, 0)]:
                if job.id in self._running:
                    continue
                self._running[job.id] = asyncio.create_task(self._run_job(job.id))

    async def _run_job(self, job_id: str) -> None:
        try:
            job = await self.get_job(job_id)
            if job.status != "queued" or self._runner is None:
                return
            running = await self._save(job, status="running", started_at=utc_now_iso())
            await self._emit_status(running, "running", "Subagent job started.")
            await self._audit_sink.record(
                "agent",
                "subagent.started",
                {"job_id": running.id, "session_id": running.session_id},
            )
            result = await self._runner(running)
            completed = await self._save(
                running,
                status="completed",
                child_run_id=result.run_id,
                final_output=result.final_output,
                completed_at=utc_now_iso(),
            )
            await self._emit_status(completed, "completed", "Subagent job completed.")
            await self._audit_sink.record(
                "agent",
                "subagent.completed",
                {
                    "job_id": completed.id,
                    "session_id": completed.session_id,
                    "child_run_id": completed.child_run_id,
                },
            )
        except asyncio.CancelledError:
            job = await self.get_job(job_id)
            await self._save(
                job,
                status="cancelled",
                completed_at=utc_now_iso(),
                error="Subagent job was cancelled.",
            )
        except Exception as exc:
            job = await self.get_job(job_id)
            failed = await self._save(
                job,
                status="failed",
                completed_at=utc_now_iso(),
                error=str(exc),
            )
            await self._emit_status(failed, "failed", str(exc))
            await self._audit_sink.record(
                "agent",
                "subagent.failed",
                {"job_id": failed.id, "session_id": failed.session_id, "error": str(exc)},
            )
        finally:
            self._running.pop(job_id, None)
            await self._schedule_queued()

    async def _save(self, job: SubagentJob, **updates: object) -> SubagentJob:
        updated = job.model_copy(update={**updates, "updated_at": utc_now_iso()})
        await self._state_store.save_subagent_job(updated)
        return updated

    async def _emit_status(
        self,
        job: SubagentJob,
        status: SubagentStatus,
        message: str,
    ) -> None:
        event = SubagentStatusEvent(
            job_id=job.id,
            status=status,
            role=job.role,
            task=job.task,
            message=message,
        )
        await self._state_store.append_event(job.parent_run_id, event)
        if self._event_observer is not None:
            self._event_observer(event)


def _subagent_statuses() -> tuple[SubagentStatus, ...]:
    return ("queued", "running", "completed", "failed", "cancelled", "interrupted")
