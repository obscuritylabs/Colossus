import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.preferences import ReplPreferencesService
from colossus.application.tasks import TaskService
from colossus.domain.errors import ColossusError
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
