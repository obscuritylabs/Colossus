"""SQLite run state store."""

import re
import sqlite3
from contextlib import closing
from pathlib import Path

from pydantic import TypeAdapter

from colossus.domain.context import ContextSnapshot
from colossus.domain.decisions import DecisionStatus, KeyDecision
from colossus.domain.events import RunEvent
from colossus.domain.integrations import IntegrationConnection
from colossus.domain.memories import MemoryItem, MemoryKind, MemoryScope, MemoryStatus
from colossus.domain.messages import Message, UserMessage
from colossus.domain.packs import InstalledPack, PackTrustRecord
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences
from colossus.domain.research import ResearchClaim, ResearchRun, ResearchSource
from colossus.domain.sessions import SessionSummary
from colossus.domain.subagents import SubagentJob, SubagentStatus
from colossus.domain.tasks import Task, TaskStatus

_RUN_EVENT_ADAPTER: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
_MESSAGE_ADAPTER: TypeAdapter[Message] = TypeAdapter(Message)
_SQL_NOW = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
_SESSION_SUMMARY_SELECT = """
    select
        s.id,
        s.title,
        s.created_at,
        coalesce(s.updated_at, s.created_at) as updated_at,
        (select count(*) from messages m where m.session_id = s.id) as message_count,
        (
            select m.run_id
            from messages m
            where m.session_id = s.id
            order by m.sequence desc
            limit 1
        ) as last_run_id,
        (
            select m.payload
            from messages m
            where m.session_id = s.id and m.role = 'user'
            order by m.sequence desc
            limit 1
        ) as last_user_payload
    from sessions s
"""


class SQLiteStateStore:
    def __init__(self, path: Path) -> None:
        self._path = path
        self._path.parent.mkdir(parents=True, exist_ok=True)
        self._init()

    async def append_event(self, run_id: str, event: RunEvent) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                "insert into run_events(run_id, event_type, payload) values (?, ?, ?)",
                (run_id, event.type, _RUN_EVENT_ADAPTER.dump_json(event).decode()),
            )
            conn.commit()

    async def list_events(self, run_id: str) -> tuple[RunEvent, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                "select payload from run_events where run_id = ? order by id",
                (run_id,),
            ).fetchall()
        return tuple(_RUN_EVENT_ADAPTER.validate_json(row[0]) for row in rows)

    async def ensure_session(self, session_id: str, title: str | None = None) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                f"""
                insert into sessions(id, title, updated_at) values (?, ?, {_SQL_NOW})
                on conflict(id) do nothing
                """,
                (session_id, title),
            )
            if title is not None:
                conn.execute(
                    """
                    update sessions
                    set title = coalesce(title, ?)
                    where id = ?
                    """,
                    (title, session_id),
                )
            conn.commit()

    async def get_session(self, session_id: str) -> SessionSummary | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                _SESSION_SUMMARY_SELECT + " where s.id = ?",
                (session_id,),
            ).fetchone()
        if row is None:
            return None
        return _session_summary_from_row(row)

    async def list_sessions(self, limit: int = 20) -> tuple[SessionSummary, ...]:
        safe_limit = min(max(limit, 1), 100)
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                _SESSION_SUMMARY_SELECT
                + """
                order by coalesce(s.updated_at, s.created_at) desc, s.id desc
                limit ?
                """,
                (safe_limit,),
            ).fetchall()
        return tuple(_session_summary_from_row(row) for row in rows)

    async def append_message(self, session_id: str, run_id: str, message: Message) -> None:
        await self.ensure_session(session_id)
        with closing(sqlite3.connect(self._path)) as conn:
            next_sequence = conn.execute(
                "select coalesce(max(sequence), 0) + 1 from messages where session_id = ?",
                (session_id,),
            ).fetchone()[0]
            conn.execute(
                """
                insert into messages(session_id, run_id, sequence, role, payload)
                values (?, ?, ?, ?, ?)
                """,
                (
                    session_id,
                    run_id,
                    next_sequence,
                    message.role,
                    _MESSAGE_ADAPTER.dump_json(message).decode(),
                ),
            )
            conn.execute(
                f"update sessions set updated_at = {_SQL_NOW} where id = ?",
                (session_id,),
            )
            conn.commit()

    async def list_messages(self, session_id: str) -> tuple[Message, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                "select payload from messages where session_id = ? order by sequence",
                (session_id,),
            ).fetchall()
        return tuple(_MESSAGE_ADAPTER.validate_json(row[0]) for row in rows)

    async def save_plan(self, plan: Plan) -> None:
        await self.ensure_session(plan.session_id)
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into plans(id, session_id, status, payload)
                values (?, ?, ?, ?)
                on conflict(id) do update set
                    session_id = excluded.session_id,
                    status = excluded.status,
                    payload = excluded.payload
                """,
                (plan.id, plan.session_id, plan.status, plan.model_dump_json()),
            )
            conn.commit()

    async def get_plan(self, plan_id: str) -> Plan | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute("select payload from plans where id = ?", (plan_id,)).fetchone()
        if row is None:
            return None
        return Plan.model_validate_json(row[0])

    async def list_plans(self, session_id: str | None = None) -> tuple[Plan, ...]:
        query = "select payload from plans"
        params: tuple[str, ...] = ()
        if session_id is not None:
            query += " where session_id = ?"
            params = (session_id,)
        query += " order by id"
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(query, params).fetchall()
        return tuple(Plan.model_validate_json(row[0]) for row in rows)

    async def save_task(self, task: Task) -> None:
        await self.ensure_session(task.session_id)
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into tasks(id, session_id, status, payload)
                values (?, ?, ?, ?)
                on conflict(id) do update set
                    session_id = excluded.session_id,
                    status = excluded.status,
                    payload = excluded.payload
                """,
                (task.id, task.session_id, task.status, task.model_dump_json()),
            )
            conn.commit()

    async def get_task(self, task_id: str) -> Task | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute("select payload from tasks where id = ?", (task_id,)).fetchone()
        if row is None:
            return None
        return Task.model_validate_json(row[0])

    async def list_tasks(
        self,
        session_id: str | None = None,
        status: TaskStatus | None = None,
    ) -> tuple[Task, ...]:
        query = "select payload from tasks"
        clauses: list[str] = []
        params: list[str] = []
        if session_id is not None:
            clauses.append("session_id = ?")
            params.append(session_id)
        if status is not None:
            clauses.append("status = ?")
            params.append(status)
        if clauses:
            query += f" where {' and '.join(clauses)}"
        query += " order by created_at, id"
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(query, tuple(params)).fetchall()
        return tuple(Task.model_validate_json(row[0]) for row in rows)

    async def save_decision(self, decision: KeyDecision) -> None:
        await self.ensure_session(decision.session_id)
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into decisions(id, session_id, status, priority, payload)
                values (?, ?, ?, ?, ?)
                on conflict(id) do update set
                    session_id = excluded.session_id,
                    status = excluded.status,
                    priority = excluded.priority,
                    payload = excluded.payload
                """,
                (
                    decision.id,
                    decision.session_id,
                    decision.status,
                    decision.priority,
                    decision.model_dump_json(),
                ),
            )
            conn.commit()

    async def get_decision(self, decision_id: str) -> KeyDecision | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from decisions where id = ?",
                (decision_id,),
            ).fetchone()
        if row is None:
            return None
        return KeyDecision.model_validate_json(row[0])

    async def list_decisions(
        self,
        session_id: str | None = None,
        status: DecisionStatus | None = None,
    ) -> tuple[KeyDecision, ...]:
        query = "select payload from decisions"
        clauses: list[str] = []
        params: list[str] = []
        if session_id is not None:
            clauses.append("session_id = ?")
            params.append(session_id)
        if status is not None:
            clauses.append("status = ?")
            params.append(status)
        if clauses:
            query += f" where {' and '.join(clauses)}"
        query += " order by priority, created_at, id"
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(query, tuple(params)).fetchall()
        return tuple(KeyDecision.model_validate_json(row[0]) for row in rows)

    async def save_memory(self, memory: MemoryItem) -> None:
        if memory.session_id is not None:
            await self.ensure_session(memory.session_id, title=memory.text[:80])
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into memories(
                    id,
                    scope,
                    kind,
                    status,
                    source,
                    repo_root,
                    session_id,
                    payload
                )
                values (?, ?, ?, ?, ?, ?, ?, ?)
                on conflict(id) do update set
                    scope = excluded.scope,
                    kind = excluded.kind,
                    status = excluded.status,
                    source = excluded.source,
                    repo_root = excluded.repo_root,
                    session_id = excluded.session_id,
                    payload = excluded.payload
                """,
                (
                    memory.id,
                    memory.scope,
                    memory.kind,
                    memory.status,
                    memory.source,
                    memory.repo_root,
                    memory.session_id,
                    memory.model_dump_json(),
                ),
            )
            conn.commit()

    async def get_memory(self, memory_id: str) -> MemoryItem | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from memories where id = ?",
                (memory_id,),
            ).fetchone()
        if row is None:
            return None
        return MemoryItem.model_validate_json(row[0])

    async def list_memories(
        self,
        scope: MemoryScope | None = None,
        kind: MemoryKind | None = None,
        status: MemoryStatus | None = None,
        repo_root: str | None = None,
        session_id: str | None = None,
    ) -> tuple[MemoryItem, ...]:
        query = "select payload from memories"
        clauses: list[str] = []
        params: list[str] = []
        if scope is not None:
            clauses.append("scope = ?")
            params.append(scope)
        if kind is not None:
            clauses.append("kind = ?")
            params.append(kind)
        if status is not None:
            clauses.append("status = ?")
            params.append(status)
        if repo_root is not None:
            clauses.append("repo_root = ?")
            params.append(repo_root)
        if session_id is not None:
            clauses.append("session_id = ?")
            params.append(session_id)
        if clauses:
            query += f" where {' and '.join(clauses)}"
        query += " order by scope, kind, created_at, id"
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(query, tuple(params)).fetchall()
        return tuple(MemoryItem.model_validate_json(row[0]) for row in rows)

    async def upsert_memory_index(self, memory: MemoryItem) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute("delete from memories_fts where memory_id = ?", (memory.id,))
            conn.execute(
                """
                insert into memories_fts(memory_id, scope, kind, text, rationale)
                values (?, ?, ?, ?, ?)
                """,
                (memory.id, memory.scope, memory.kind, memory.text, memory.rationale),
            )
            conn.commit()

    async def delete_memory_index(self, memory_id: str) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute("delete from memories_fts where memory_id = ?", (memory_id,))
            conn.commit()

    async def search_memory_index(self, query: str, *, limit: int = 20) -> tuple[str, ...]:
        match_query = _memory_match_query(query)
        if not match_query:
            return ()
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                """
                select memory_id
                from memories_fts
                where memories_fts match ?
                order by bm25(memories_fts), memory_id
                limit ?
                """,
                (match_query, limit),
            ).fetchall()
        return tuple(row[0] for row in rows)

    async def save_research_run(self, run: ResearchRun) -> None:
        await self.ensure_session(run.session_id, title=run.question[:80])
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into research_runs(id, session_id, status, payload)
                values (?, ?, ?, ?)
                on conflict(id) do update set
                    session_id = excluded.session_id,
                    status = excluded.status,
                    payload = excluded.payload
                """,
                (run.id, run.session_id, run.status, run.model_dump_json()),
            )
            conn.commit()

    async def get_research_run(self, run_id: str) -> ResearchRun | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from research_runs where id = ?",
                (run_id,),
            ).fetchone()
        if row is None:
            return None
        return ResearchRun.model_validate_json(row[0])

    async def list_research_runs(self, session_id: str | None = None) -> tuple[ResearchRun, ...]:
        query = "select payload from research_runs"
        params: tuple[str, ...] = ()
        if session_id is not None:
            query += " where session_id = ?"
            params = (session_id,)
        query += " order by created_at desc, id desc"
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(query, params).fetchall()
        return tuple(ResearchRun.model_validate_json(row[0]) for row in rows)

    async def save_research_source(self, source: ResearchSource) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into research_sources(id, run_id, label, kind, payload)
                values (?, ?, ?, ?, ?)
                on conflict(id) do update set
                    run_id = excluded.run_id,
                    label = excluded.label,
                    kind = excluded.kind,
                    payload = excluded.payload
                """,
                (
                    source.id,
                    source.run_id,
                    source.label,
                    source.kind,
                    source.model_dump_json(),
                ),
            )
            conn.commit()

    async def list_research_sources(self, run_id: str) -> tuple[ResearchSource, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                """
                select payload from research_sources
                where run_id = ?
                order by cast(substr(label, 2) as integer), label
                """,
                (run_id,),
            ).fetchall()
        return tuple(ResearchSource.model_validate_json(row[0]) for row in rows)

    async def save_research_claim(self, claim: ResearchClaim) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into research_claims(id, run_id, payload)
                values (?, ?, ?)
                on conflict(id) do update set
                    run_id = excluded.run_id,
                    payload = excluded.payload
                """,
                (claim.id, claim.run_id, claim.model_dump_json()),
            )
            conn.commit()

    async def list_research_claims(self, run_id: str) -> tuple[ResearchClaim, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                "select payload from research_claims where run_id = ? order by id",
                (run_id,),
            ).fetchall()
        return tuple(ResearchClaim.model_validate_json(row[0]) for row in rows)

    async def save_subagent_job(self, job: SubagentJob) -> None:
        await self.ensure_session(job.session_id)
        await self.ensure_session(job.child_session_id, title=f"subagent {job.id}")
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into subagent_jobs(id, session_id, status, payload)
                values (?, ?, ?, ?)
                on conflict(id) do update set
                    session_id = excluded.session_id,
                    status = excluded.status,
                    payload = excluded.payload
                """,
                (job.id, job.session_id, job.status, job.model_dump_json()),
            )
            conn.commit()

    async def get_subagent_job(self, job_id: str) -> SubagentJob | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from subagent_jobs where id = ?",
                (job_id,),
            ).fetchone()
        if row is None:
            return None
        return SubagentJob.model_validate_json(row[0])

    async def list_subagent_jobs(
        self,
        session_id: str | None = None,
        status: SubagentStatus | None = None,
    ) -> tuple[SubagentJob, ...]:
        query = "select payload from subagent_jobs"
        clauses: list[str] = []
        params: list[str] = []
        if session_id is not None:
            clauses.append("session_id = ?")
            params.append(session_id)
        if status is not None:
            clauses.append("status = ?")
            params.append(status)
        if clauses:
            query += f" where {' and '.join(clauses)}"
        query += " order by created_at, id"
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(query, tuple(params)).fetchall()
        return tuple(SubagentJob.model_validate_json(row[0]) for row in rows)

    async def save_integration_connection(self, connection: IntegrationConnection) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into integration_connections(name, kind, status, payload)
                values (?, ?, ?, ?)
                on conflict(name) do update set
                    kind = excluded.kind,
                    status = excluded.status,
                    payload = excluded.payload
                """,
                (
                    connection.name,
                    connection.kind,
                    connection.status,
                    connection.model_dump_json(),
                ),
            )
            conn.commit()

    async def get_integration_connection(
        self,
        name: str,
    ) -> IntegrationConnection | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from integration_connections where name = ?",
                (name,),
            ).fetchone()
        if row is None:
            return None
        return IntegrationConnection.model_validate_json(row[0])

    async def list_integration_connections(self) -> tuple[IntegrationConnection, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                """
                select payload
                from integration_connections
                order by name
                """
            ).fetchall()
        return tuple(IntegrationConnection.model_validate_json(row[0]) for row in rows)

    async def delete_integration_connection(self, name: str) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute("delete from integration_connections where name = ?", (name,))
            conn.commit()

    async def save_installed_pack(self, pack: InstalledPack) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into installed_packs(name, version, status, payload)
                values (?, ?, ?, ?)
                on conflict(name) do update set
                    version = excluded.version,
                    status = excluded.status,
                    payload = excluded.payload,
                    updated_at = current_timestamp
                """,
                (pack.name, pack.version, pack.status, pack.model_dump_json()),
            )
            conn.commit()

    async def get_installed_pack(self, name: str) -> InstalledPack | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from installed_packs where name = ?",
                (name,),
            ).fetchone()
        if row is None:
            return None
        return InstalledPack.model_validate_json(row[0])

    async def list_installed_packs(self) -> tuple[InstalledPack, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                """
                select payload
                from installed_packs
                order by name
                """
            ).fetchall()
        return tuple(InstalledPack.model_validate_json(row[0]) for row in rows)

    async def delete_installed_pack(self, name: str) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute("delete from installed_packs where name = ?", (name,))
            conn.commit()

    async def save_pack_trust_record(self, record: PackTrustRecord) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into pack_trust_records(kind, value, payload)
                values (?, ?, ?)
                on conflict(kind, value) do update set
                    payload = excluded.payload
                """,
                (record.kind, record.value, record.model_dump_json()),
            )
            conn.commit()

    async def list_pack_trust_records(self) -> tuple[PackTrustRecord, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                """
                select payload
                from pack_trust_records
                order by kind, value
                """
            ).fetchall()
        return tuple(PackTrustRecord.model_validate_json(row[0]) for row in rows)

    async def save_context_snapshot(self, snapshot: ContextSnapshot) -> None:
        await self.ensure_session(snapshot.session_id)
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into context_snapshots(
                    id,
                    session_id,
                    source_start,
                    source_end,
                    payload
                )
                values (?, ?, ?, ?, ?)
                on conflict(id) do update set
                    session_id = excluded.session_id,
                    source_start = excluded.source_start,
                    source_end = excluded.source_end,
                    payload = excluded.payload
                """,
                (
                    snapshot.id,
                    snapshot.session_id,
                    snapshot.source_message_range[0],
                    snapshot.source_message_range[1],
                    snapshot.model_dump_json(),
                ),
            )
            conn.execute(
                "update sessions set active_context_snapshot_id = ? where id = ?",
                (snapshot.id, snapshot.session_id),
            )
            conn.commit()

    async def get_context_snapshot(self, snapshot_id: str) -> ContextSnapshot | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from context_snapshots where id = ?",
                (snapshot_id,),
            ).fetchone()
        if row is None:
            return None
        return ContextSnapshot.model_validate_json(row[0])

    async def latest_context_snapshot(self, session_id: str) -> ContextSnapshot | None:
        with closing(sqlite3.connect(self._path)) as conn:
            active = conn.execute(
                "select active_context_snapshot_id from sessions where id = ?",
                (session_id,),
            ).fetchone()
            if active is not None and active[0] is not None:
                row = conn.execute(
                    "select payload from context_snapshots where id = ?",
                    (active[0],),
                ).fetchone()
                if row is not None:
                    return ContextSnapshot.model_validate_json(row[0])
            row = conn.execute(
                """
                select payload from context_snapshots
                where session_id = ?
                order by source_end desc, created_at desc, id desc
                limit 1
                """,
                (session_id,),
            ).fetchone()
        if row is None:
            return None
        return ContextSnapshot.model_validate_json(row[0])

    async def list_context_snapshots(self, session_id: str) -> tuple[ContextSnapshot, ...]:
        with closing(sqlite3.connect(self._path)) as conn:
            rows = conn.execute(
                """
                select payload from context_snapshots
                where session_id = ?
                order by source_end desc, created_at desc, id desc
                """,
                (session_id,),
            ).fetchall()
        return tuple(ContextSnapshot.model_validate_json(row[0]) for row in rows)

    async def restore_context_snapshot(self, snapshot_id: str) -> ContextSnapshot:
        snapshot = await self.get_context_snapshot(snapshot_id)
        if snapshot is None:
            raise ValueError(f"Context snapshot not found: {snapshot_id}")
        await self.ensure_session(snapshot.session_id)
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                "update sessions set active_context_snapshot_id = ? where id = ?",
                (snapshot_id, snapshot.session_id),
            )
            conn.commit()
        return snapshot

    async def get_repl_preferences(self, profile: str) -> ReplPreferences | None:
        with closing(sqlite3.connect(self._path)) as conn:
            row = conn.execute(
                "select payload from repl_preferences where profile = ?",
                (profile,),
            ).fetchone()
        if row is None:
            return None
        return ReplPreferences.model_validate_json(row[0])

    async def save_repl_preferences(
        self,
        profile: str,
        preferences: ReplPreferences,
    ) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                insert into repl_preferences(profile, payload)
                values (?, ?)
                on conflict(profile) do update set
                    payload = excluded.payload,
                    updated_at = current_timestamp
                """,
                (profile, preferences.model_dump_json()),
            )
            conn.commit()

    def _init(self) -> None:
        with closing(sqlite3.connect(self._path)) as conn:
            conn.execute(
                """
                create table if not exists run_events (
                    id integer primary key autoincrement,
                    run_id text not null,
                    event_type text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute("create index if not exists idx_run_events_run_id on run_events(run_id)")
            conn.execute(
                """
                create table if not exists sessions (
                    id text primary key,
                    title text,
                    active_context_snapshot_id text,
                    updated_at datetime default current_timestamp,
                    created_at datetime default current_timestamp
                )
                """
            )
            _ensure_column(conn, "sessions", "active_context_snapshot_id", "text")
            _ensure_column(conn, "sessions", "updated_at", "datetime")
            conn.execute(
                """
                update sessions
                set updated_at = coalesce(updated_at, created_at, current_timestamp)
                """
            )
            conn.execute(
                """
                create table if not exists messages (
                    id integer primary key autoincrement,
                    session_id text not null,
                    run_id text not null,
                    sequence integer not null,
                    role text not null,
                    payload text not null,
                    created_at datetime default current_timestamp,
                    unique(session_id, sequence)
                )
                """
            )
            conn.execute("create index if not exists idx_messages_session on messages(session_id)")
            conn.execute(
                """
                create table if not exists plans (
                    id text primary key,
                    session_id text not null,
                    status text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute("create index if not exists idx_plans_session on plans(session_id)")
            conn.execute(
                """
                create table if not exists tasks (
                    id text primary key,
                    session_id text not null,
                    status text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute("create index if not exists idx_tasks_session on tasks(session_id)")
            conn.execute("create index if not exists idx_tasks_status on tasks(status)")
            conn.execute(
                """
                create table if not exists decisions (
                    id text primary key,
                    session_id text not null,
                    status text not null,
                    priority text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_decisions_session on decisions(session_id)"
            )
            conn.execute("create index if not exists idx_decisions_status on decisions(status)")
            conn.execute(
                """
                create table if not exists memories (
                    id text primary key,
                    scope text not null,
                    kind text not null,
                    status text not null,
                    source text not null,
                    repo_root text,
                    session_id text,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute("create index if not exists idx_memories_scope on memories(scope)")
            conn.execute("create index if not exists idx_memories_kind on memories(kind)")
            conn.execute("create index if not exists idx_memories_status on memories(status)")
            conn.execute(
                "create index if not exists idx_memories_repo on memories(repo_root)"
            )
            conn.execute(
                "create index if not exists idx_memories_session on memories(session_id)"
            )
            conn.execute(
                """
                create virtual table if not exists memories_fts
                using fts5(memory_id unindexed, scope, kind, text, rationale)
                """
            )
            conn.execute(
                """
                create table if not exists research_runs (
                    id text primary key,
                    session_id text not null,
                    status text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_research_runs_session "
                "on research_runs(session_id)"
            )
            conn.execute(
                "create index if not exists idx_research_runs_status "
                "on research_runs(status)"
            )
            conn.execute(
                """
                create table if not exists research_sources (
                    id text primary key,
                    run_id text not null,
                    label text not null,
                    kind text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_research_sources_run "
                "on research_sources(run_id)"
            )
            conn.execute(
                """
                create table if not exists research_claims (
                    id text primary key,
                    run_id text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_research_claims_run "
                "on research_claims(run_id)"
            )
            conn.execute(
                """
                create table if not exists subagent_jobs (
                    id text primary key,
                    session_id text not null,
                    status text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_subagent_jobs_session "
                "on subagent_jobs(session_id)"
            )
            conn.execute(
                "create index if not exists idx_subagent_jobs_status "
                "on subagent_jobs(status)"
            )
            conn.execute(
                """
                create table if not exists integration_connections (
                    name text primary key,
                    kind text not null,
                    status text not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_integration_connections_kind "
                "on integration_connections(kind)"
            )
            conn.execute(
                "create index if not exists idx_integration_connections_status "
                "on integration_connections(status)"
            )
            conn.execute(
                """
                create table if not exists installed_packs (
                    name text primary key,
                    version text not null,
                    status text not null,
                    payload text not null,
                    updated_at datetime default current_timestamp,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                "create index if not exists idx_installed_packs_status "
                "on installed_packs(status)"
            )
            conn.execute(
                """
                create table if not exists pack_trust_records (
                    kind text not null,
                    value text not null,
                    payload text not null,
                    created_at datetime default current_timestamp,
                    primary key(kind, value)
                )
                """
            )
            conn.execute(
                """
                create table if not exists context_snapshots (
                    id text primary key,
                    session_id text not null,
                    source_start integer not null,
                    source_end integer not null,
                    payload text not null,
                    created_at datetime default current_timestamp
                )
                """
            )
            conn.execute(
                """
                create index if not exists idx_context_snapshots_session
                on context_snapshots(session_id, source_end)
                """
            )
            conn.execute(
                """
                create table if not exists repl_preferences (
                    profile text primary key,
                    payload text not null,
                    updated_at datetime default current_timestamp
                )
                """
            )
            conn.commit()


def _ensure_column(conn: sqlite3.Connection, table: str, column: str, definition: str) -> None:
    rows = conn.execute(f"pragma table_info({table})").fetchall()
    if column in {row[1] for row in rows}:
        return
    conn.execute(f"alter table {table} add column {column} {definition}")


def _session_summary_from_row(row: sqlite3.Row | tuple[object, ...]) -> SessionSummary:
    last_user_payload = row[6]
    preview = _last_user_preview(last_user_payload) if isinstance(last_user_payload, str) else None
    raw_message_count = row[4]
    message_count = raw_message_count if isinstance(raw_message_count, int) else 0
    return SessionSummary(
        id=str(row[0]),
        title=str(row[1]) if row[1] is not None else None,
        created_at=str(row[2]),
        updated_at=str(row[3]),
        message_count=message_count,
        last_run_id=str(row[5]) if row[5] is not None else None,
        last_user_preview=preview,
    )


def _last_user_preview(payload: str) -> str | None:
    message = _MESSAGE_ADAPTER.validate_json(payload)
    if not isinstance(message, UserMessage):
        return None
    return _short_preview(message.content)


def _short_preview(value: str, limit: int = 120) -> str:
    normalized = " ".join(value.split())
    if len(normalized) <= limit:
        return normalized
    return normalized[: limit - 3].rstrip() + "..."


def _memory_match_query(query: str) -> str:
    terms = re.findall(r"[A-Za-z0-9_]+", query.lower())[:12]
    return " OR ".join(f'"{term}"' for term in terms)
