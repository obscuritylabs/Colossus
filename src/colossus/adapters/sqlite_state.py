"""SQLite run state store."""

import re
import sqlite3
from contextlib import closing
from pathlib import Path

from pydantic import TypeAdapter

from colossus.domain.context import ContextSnapshot
from colossus.domain.decisions import DecisionStatus, KeyDecision
from colossus.domain.events import RunEvent
from colossus.domain.memories import MemoryItem, MemoryKind, MemoryScope, MemoryStatus
from colossus.domain.messages import Message
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences
from colossus.domain.subagents import SubagentJob, SubagentStatus
from colossus.domain.tasks import Task, TaskStatus

_RUN_EVENT_ADAPTER: TypeAdapter[RunEvent] = TypeAdapter(RunEvent)
_MESSAGE_ADAPTER: TypeAdapter[Message] = TypeAdapter(Message)


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
                """
                insert into sessions(id, title) values (?, ?)
                on conflict(id) do nothing
                """,
                (session_id, title),
            )
            conn.commit()

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
                    created_at datetime default current_timestamp
                )
                """
            )
            _ensure_column(conn, "sessions", "active_context_snapshot_id", "text")
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


def _memory_match_query(query: str) -> str:
    terms = re.findall(r"[A-Za-z0-9_]+", query.lower())[:12]
    return " OR ".join(f'"{term}"' for term in terms)
