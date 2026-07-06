"""State store port."""

from typing import Protocol

from colossus.domain.context import ContextSnapshot
from colossus.domain.decisions import DecisionStatus, KeyDecision
from colossus.domain.events import RunEvent
from colossus.domain.goals import Goal, GoalStatus
from colossus.domain.integrations import IntegrationConnection
from colossus.domain.memories import MemoryItem, MemoryKind, MemoryScope, MemoryStatus
from colossus.domain.messages import Message
from colossus.domain.packs import InstalledPack, PackTrustRecord
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences
from colossus.domain.research import ResearchClaim, ResearchRun, ResearchSource
from colossus.domain.sessions import SessionSummary
from colossus.domain.subagents import SubagentJob, SubagentStatus
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

    async def get_session(self, session_id: str) -> SessionSummary | None:
        """Load a session summary by id."""
        ...

    async def list_sessions(self, limit: int = 20) -> tuple[SessionSummary, ...]:
        """List session summaries ordered by most recent activity."""
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

    async def save_goal(self, goal: Goal) -> None:
        """Persist a goal-mode record."""
        ...

    async def get_goal(self, goal_id: str) -> Goal | None:
        """Load a goal by id."""
        ...

    async def list_goals(
        self,
        session_id: str | None = None,
        status: GoalStatus | None = None,
    ) -> tuple[Goal, ...]:
        """List goals, optionally scoped to a session and status."""
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

    async def save_decision(self, decision: KeyDecision) -> None:
        """Persist a key decision."""
        ...

    async def get_decision(self, decision_id: str) -> KeyDecision | None:
        """Load a key decision by id."""
        ...

    async def list_decisions(
        self,
        session_id: str | None = None,
        status: DecisionStatus | None = None,
    ) -> tuple[KeyDecision, ...]:
        """List key decisions, optionally scoped to a session and status."""
        ...

    async def save_memory(self, memory: MemoryItem) -> None:
        """Persist a durable memory."""
        ...

    async def get_memory(self, memory_id: str) -> MemoryItem | None:
        """Load a durable memory by id."""
        ...

    async def list_memories(
        self,
        scope: MemoryScope | None = None,
        kind: MemoryKind | None = None,
        status: MemoryStatus | None = None,
        repo_root: str | None = None,
        session_id: str | None = None,
    ) -> tuple[MemoryItem, ...]:
        """List memories with optional scope, kind, status, and owner filters."""
        ...

    async def save_research_run(self, run: ResearchRun) -> None:
        """Persist a research run."""
        ...

    async def get_research_run(self, run_id: str) -> ResearchRun | None:
        """Load a research run by id."""
        ...

    async def list_research_runs(self, session_id: str | None = None) -> tuple[ResearchRun, ...]:
        """List research runs, optionally scoped to a session."""
        ...

    async def save_research_source(self, source: ResearchSource) -> None:
        """Persist a research source."""
        ...

    async def list_research_sources(self, run_id: str) -> tuple[ResearchSource, ...]:
        """List sources for a research run."""
        ...

    async def save_research_claim(self, claim: ResearchClaim) -> None:
        """Persist a research claim."""
        ...

    async def list_research_claims(self, run_id: str) -> tuple[ResearchClaim, ...]:
        """List claims for a research run."""
        ...

    async def save_subagent_job(self, job: SubagentJob) -> None:
        """Persist a subagent job."""
        ...

    async def get_subagent_job(self, job_id: str) -> SubagentJob | None:
        """Load a subagent job by id."""
        ...

    async def list_subagent_jobs(
        self,
        session_id: str | None = None,
        status: SubagentStatus | None = None,
    ) -> tuple[SubagentJob, ...]:
        """List subagent jobs, optionally scoped to a session and status."""
        ...

    async def save_integration_connection(self, connection: IntegrationConnection) -> None:
        """Persist an integration connection record."""
        ...

    async def get_integration_connection(
        self,
        name: str,
    ) -> IntegrationConnection | None:
        """Load an integration connection by name."""
        ...

    async def list_integration_connections(self) -> tuple[IntegrationConnection, ...]:
        """List persisted integration connection records."""
        ...

    async def delete_integration_connection(self, name: str) -> None:
        """Delete an integration connection record."""
        ...

    async def save_installed_pack(self, pack: InstalledPack) -> None:
        """Persist an installed pack record."""
        ...

    async def get_installed_pack(self, name: str) -> InstalledPack | None:
        """Load an installed pack record by name."""
        ...

    async def list_installed_packs(self) -> tuple[InstalledPack, ...]:
        """List installed pack records."""
        ...

    async def delete_installed_pack(self, name: str) -> None:
        """Delete an installed pack record."""
        ...

    async def save_pack_trust_record(self, record: PackTrustRecord) -> None:
        """Persist a pack trust record."""
        ...

    async def list_pack_trust_records(self) -> tuple[PackTrustRecord, ...]:
        """List pack trust records."""
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
