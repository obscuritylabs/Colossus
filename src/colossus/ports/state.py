"""State store port."""

from typing import Protocol

from colossus.domain.context import ContextSnapshot
from colossus.domain.events import RunEvent
from colossus.domain.messages import Message
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences
from colossus.domain.tasks import Task, TaskStatus


class StateStore(Protocol):
    async def append_event(self, run_id: str, event: RunEvent) -> None:
        """Persist a run event."""
        ...

    async def list_events(self, run_id: str) -> tuple[RunEvent, ...]:
        """Load events for a run."""
        ...

    async def ensure_session(self, session_id: str, title: str | None = None) -> None:
        """Create a session if it does not already exist."""
        ...

    async def append_message(self, session_id: str, run_id: str, message: Message) -> None:
        """Persist a normalized conversation message."""
        ...

    async def list_messages(self, session_id: str) -> tuple[Message, ...]:
        """Load normalized conversation messages for a session."""
        ...

    async def save_plan(self, plan: Plan) -> None:
        """Persist a plan."""
        ...

    async def get_plan(self, plan_id: str) -> Plan | None:
        """Load a plan by id."""
        ...

    async def list_plans(self, session_id: str | None = None) -> tuple[Plan, ...]:
        """List plans, optionally scoped to a session."""
        ...

    async def save_task(self, task: Task) -> None:
        """Persist a session task."""
        ...

    async def get_task(self, task_id: str) -> Task | None:
        """Load a task by id."""
        ...

    async def list_tasks(
        self,
        session_id: str | None = None,
        status: TaskStatus | None = None,
    ) -> tuple[Task, ...]:
        """List tasks, optionally scoped to a session and status."""
        ...

    async def save_context_snapshot(self, snapshot: ContextSnapshot) -> None:
        """Persist a context snapshot."""
        ...

    async def get_context_snapshot(self, snapshot_id: str) -> ContextSnapshot | None:
        """Load a context snapshot by id."""
        ...

    async def latest_context_snapshot(self, session_id: str) -> ContextSnapshot | None:
        """Load the active or newest context snapshot for a session."""
        ...

    async def list_context_snapshots(self, session_id: str) -> tuple[ContextSnapshot, ...]:
        """List context snapshots for a session."""
        ...

    async def restore_context_snapshot(self, snapshot_id: str) -> ContextSnapshot:
        """Mark a context snapshot as active for future context builds."""
        ...

    async def get_repl_preferences(self, profile: str) -> ReplPreferences | None:
        """Load persisted REPL preferences."""
        ...

    async def save_repl_preferences(
        self,
        profile: str,
        preferences: ReplPreferences,
    ) -> None:
        """Persist REPL preferences."""
        ...
