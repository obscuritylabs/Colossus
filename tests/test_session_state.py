import sqlite3

import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.decisions import DecisionService
from colossus.application.memories import MemoryService
from colossus.application.preferences import ReplPreferencesService
from colossus.application.tasks import TaskService
from colossus.domain.decisions import KeyDecision
from colossus.domain.errors import ColossusError
from colossus.domain.memories import MemoryItem
from colossus.domain.messages import AssistantMessage, UserMessage
from colossus.domain.preferences import ReplPreferences
from colossus.domain.tasks import Task


@pytest.mark.asyncio
async def test_sqlite_state_persists_session_messages(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")

    await state.append_message("session-1", "run-1", UserMessage(content="hello"))
    await state.append_message("session-1", "run-1", AssistantMessage(content="hi"))

    messages = await state.list_messages("session-1")
    assert [message.role for message in messages] == ["user", "assistant"]
    assert messages[0].content == "hello"


@pytest.mark.asyncio
async def test_sqlite_state_lists_session_summaries_by_recent_activity(tmp_path) -> None:
    path = tmp_path / "state.sqlite3"
    state = SQLiteStateStore(path)

    await state.append_message("session-old", "run-old", UserMessage(content="older question"))
    await state.append_message("session-old", "run-old", AssistantMessage(content="older answer"))
    await state.append_message(
        "session-new",
        "run-new",
        UserMessage(content="newer question with enough detail to preview"),
    )
    with sqlite3.connect(path) as conn:
        conn.execute(
            "update sessions set updated_at = ? where id = ?",
            ("2026-01-01", "session-old"),
        )
        conn.execute(
            "update sessions set updated_at = ? where id = ?",
            ("2026-01-02", "session-new"),
        )

    sessions = await state.list_sessions(limit=10)
    latest = await state.get_session("session-new")

    assert [session.id for session in sessions[:2]] == ["session-new", "session-old"]
    assert latest is not None
    assert latest.message_count == 1
    assert latest.last_run_id == "run-new"
    assert latest.last_user_preview == "newer question with enough detail to preview"


@pytest.mark.asyncio
async def test_sqlite_state_migrates_sessions_without_updated_at(tmp_path) -> None:
    path = tmp_path / "state.sqlite3"
    with sqlite3.connect(path) as conn:
        conn.execute(
            """
            create table sessions (
                id text primary key,
                title text,
                active_context_snapshot_id text,
                created_at datetime default current_timestamp
            )
            """
        )
        conn.execute(
            "insert into sessions(id, title, created_at) values (?, ?, ?)",
            ("legacy-session", "Legacy", "2026-01-01"),
        )

    state = SQLiteStateStore(path)
    session = await state.get_session("legacy-session")

    assert session is not None
    assert session.updated_at == "2026-01-01"


@pytest.mark.asyncio
async def test_sqlite_state_persists_session_tasks(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    task = Task(
        id="task-1",
        session_id="session-1",
        title="Show tasks",
        status="pending",
        created_at="2026-06-10T00:00:00+00:00",
        updated_at="2026-06-10T00:00:00+00:00",
    )

    await state.save_task(task)

    reloaded = SQLiteStateStore(tmp_path / "state.sqlite3")
    assert await reloaded.get_task("task-1") == task
    assert await reloaded.list_tasks(session_id="session-1") == (task,)
    assert await reloaded.list_tasks(session_id="session-1", status="completed") == ()


@pytest.mark.asyncio
async def test_sqlite_state_persists_key_decisions(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    decision = KeyDecision(
        id="kd_1",
        session_id="session-1",
        source="user",
        priority="critical",
        title="Durable commitments",
        decision="Key decisions are durable commitments, not memories.",
    )

    await state.save_decision(decision)

    reloaded = SQLiteStateStore(tmp_path / "state.sqlite3")
    assert await reloaded.get_decision("kd_1") == decision
    assert await reloaded.list_decisions(session_id="session-1") == (decision,)
    assert await reloaded.list_decisions(session_id="session-1", status="archived") == ()


@pytest.mark.asyncio
async def test_sqlite_state_persists_memories_and_fts_index(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    memory = MemoryItem(
        id="mem_1",
        scope="repo",
        kind="project_fact",
        source="user",
        text="Colossus stores durable memories in SQLite FTS.",
        repo_root="/repo",
    )

    await state.save_memory(memory)
    await state.upsert_memory_index(memory)

    reloaded = SQLiteStateStore(tmp_path / "state.sqlite3")
    assert await reloaded.get_memory("mem_1") == memory
    assert await reloaded.list_memories(scope="repo", repo_root="/repo") == (memory,)
    assert await reloaded.search_memory_index("durable memories", limit=5) == ("mem_1",)


@pytest.mark.asyncio
async def test_task_service_creates_updates_and_scopes_tasks(tmp_path) -> None:
    service = TaskService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )

    created = await service.create_task(
        session_id="session-1",
        title="Add UX",
        description="Render task list",
    )
    await service.create_task(session_id="session-2", title="Different session")
    updated = await service.update_task(created.id, status="completed")

    session_tasks = await service.list_tasks(session_id="session-1")
    completed = await service.list_tasks(session_id="session-1", status="completed")

    assert updated.status == "completed"
    assert [task.id for task in session_tasks] == [created.id]
    assert completed == (updated,)


@pytest.mark.asyncio
async def test_task_service_rejects_cross_session_update(tmp_path) -> None:
    service = TaskService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    created = await service.create_task(session_id="session-1", title="Scoped")

    with pytest.raises(ColossusError, match="does not belong to session"):
        await service.update_task(created.id, session_id="session-2", status="completed")


@pytest.mark.asyncio
async def test_decision_service_lifecycle_and_audit(tmp_path) -> None:
    audit_path = tmp_path / "audit.jsonl"
    service = DecisionService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(audit_path),
    )

    created = await service.create_decision(
        session_id="session-1",
        title="Never forget",
        decision="Preserve key decisions across compaction.",
        source="agent",
        priority="critical",
    )
    await service.create_decision(
        session_id="session-2",
        title="Other session",
        decision="Do not show in session one.",
        source="user",
    )
    archived = await service.archive_decision(created.id, session_id="session-1")
    replacement = await service.supersede_decision(
        archived.id,
        session_id="session-1",
        title="Replacement",
        decision="Inject active key decisions before snapshots.",
        source="user",
        priority="high",
    )

    session_decisions = await service.list_decisions(session_id="session-1", status=None)
    active = await service.list_decisions(session_id="session-1")

    assert archived.status == "archived"
    assert replacement.supersedes == created.id
    assert [decision.id for decision in active] == [replacement.id]
    assert {decision.status for decision in session_decisions} == {"superseded", "active"}
    audit_text = audit_path.read_text(encoding="utf-8")
    assert "decision.created" in audit_text
    assert "decision.archived" in audit_text
    assert "decision.superseded" in audit_text


@pytest.mark.asyncio
async def test_decision_service_rejects_cross_session_archive(tmp_path) -> None:
    service = DecisionService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    created = await service.create_decision(
        session_id="session-1",
        title="Scoped",
        decision="Stay scoped.",
    )

    with pytest.raises(ColossusError, match="does not belong to session"):
        await service.archive_decision(created.id, session_id="session-2")


@pytest.mark.asyncio
async def test_memory_service_lifecycle_search_scope_and_audit(tmp_path) -> None:
    audit_path = tmp_path / "audit.jsonl"
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = MemoryService(state, JsonlAuditSink(audit_path), state)

    created = await service.create_memory(
        scope="repo",
        kind="preference",
        text="Run pytest, ruff, and mypy before calling work complete.",
        source="user",
        repo_root="/repo",
    )
    await service.create_memory(
        scope="repo",
        kind="project_fact",
        text="A different repository memory.",
        source="agent",
        repo_root="/other",
    )
    global_memory = await service.create_memory(
        scope="global",
        kind="capability",
        text="Colossus can search durable memories with SQLite FTS.",
        source="agent",
    )

    search = await service.search_memories("pytest ruff", repo_root="/repo")
    missing = await service.search_memories("pytest ruff", repo_root="/other")
    archived = await service.archive_memory(created.id)
    replacement = await service.supersede_memory(
        global_memory.id,
        text="Colossus memory search uses SQLite FTS in V1.",
        source="user",
    )

    assert [memory.id for memory in search] == [created.id]
    assert missing == ()
    assert archived.status == "archived"
    assert replacement.supersedes == global_memory.id
    assert await service.search_memories("pytest ruff", repo_root="/repo") == ()
    audit_text = audit_path.read_text(encoding="utf-8")
    assert "memory.created" in audit_text
    assert "memory.archived" in audit_text
    assert "memory.superseded" in audit_text


@pytest.mark.asyncio
async def test_sqlite_state_persists_repl_preferences(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    preferences = ReplPreferences(
        theme="carrot",
        multiline=True,
        stream_model_output=False,
        events_mode="verbose",
        show_reasoning=False,
        transcript_style="compact",
    )

    await state.save_repl_preferences("default", preferences)

    reloaded = SQLiteStateStore(tmp_path / "state.sqlite3")
    assert await reloaded.get_repl_preferences("default") == preferences


@pytest.mark.asyncio
async def test_repl_preferences_service_defaults_saves_and_resets(tmp_path) -> None:
    service = ReplPreferencesService(SQLiteStateStore(tmp_path / "state.sqlite3"))

    assert await service.load() == ReplPreferences()
    saved = await service.save(
        ReplPreferences(theme="mono", events_mode="off", transcript_style="compact")
    )
    assert await service.load() == saved
    assert await service.reset() == ReplPreferences()
    assert await service.load() == ReplPreferences()
