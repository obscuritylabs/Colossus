import pytest

from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.preferences import ReplPreferencesService
from colossus.domain.messages import AssistantMessage, UserMessage
from colossus.domain.preferences import ReplPreferences


@pytest.mark.asyncio
async def test_sqlite_state_persists_session_messages(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")

    await state.append_message("session-1", "run-1", UserMessage(content="hello"))
    await state.append_message("session-1", "run-1", AssistantMessage(content="hi"))

    messages = await state.list_messages("session-1")
    assert [message.role for message in messages] == ["user", "assistant"]
    assert messages[0].content == "hello"


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
