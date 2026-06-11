from collections.abc import AsyncIterator
from pathlib import Path

import pytest
from pydantic import ValidationError

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.approvals import AllowAllApprovalHandler
from colossus.application.context import ContextService
from colossus.application.defaults import default_agent
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.context import ContextConfig, ContextSnapshot
from colossus.domain.events import FinalOutputEvent, ModelDeltaEvent, RunEvent
from colossus.domain.messages import AssistantMessage, ToolResultMessage, UserMessage
from colossus.domain.requests import AgentRunRequest, ModelRequest


class CapturingProvider:
    name = "capturing"

    def __init__(self, text: str = "done", fail: bool = False) -> None:
        self.text = text
        self.fail = fail
        self.requests: list[ModelRequest] = []

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        self.requests.append(request)
        if self.fail:
            raise RuntimeError("provider failed")
        yield ModelDeltaEvent(text=self.text)
        yield FinalOutputEvent(text=self.text)


def _context_service(
    tmp_path: Path,
    *,
    config: ContextConfig | None = None,
) -> tuple[ContextService, SQLiteStateStore]:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ContextService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        config=config,
        model_context_windows={"model-a": 1024},
        snapshot_id_factory=lambda: "snapshot-1",
    )
    return service, state


def test_context_config_validates_percentages() -> None:
    assert ContextConfig().compact_at_percent == 0.70

    with pytest.raises(ValidationError):
        ContextConfig(compact_at_percent=0.4, target_percent=0.5)


def test_context_budget_uses_model_window_override(tmp_path: Path) -> None:
    service, _state = _context_service(tmp_path)

    assert service.context_window_tokens("model-a") == 1024
    assert service.context_window_tokens("unknown") == 32_768
    assert service.threshold_tokens("model-a") == 716


@pytest.mark.asyncio
async def test_sqlite_state_persists_and_restores_context_snapshots(tmp_path: Path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    snapshot = ContextSnapshot(
        id="snapshot-1",
        session_id="session-1",
        source_message_range=(1, 2),
        summary="summary",
    )

    await state.save_context_snapshot(snapshot)
    restored = await state.restore_context_snapshot("snapshot-1")

    assert restored == snapshot
    assert await state.get_context_snapshot("snapshot-1") == snapshot
    assert await state.latest_context_snapshot("session-1") == snapshot
    assert await state.list_context_snapshots("session-1") == (snapshot,)


@pytest.mark.asyncio
async def test_context_prepare_does_not_compact_below_threshold(tmp_path: Path) -> None:
    service, _state = _context_service(
        tmp_path,
        config=ContextConfig(
            default_context_window_tokens=1024,
            compact_at_percent=0.8,
            target_percent=0.5,
        ),
    )
    messages = (UserMessage(content="short prompt"),)

    result = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=messages,
    )

    assert result.compacted is False
    assert result.messages == messages


@pytest.mark.asyncio
async def test_context_prepare_auto_compacts_and_preserves_raw_messages(tmp_path: Path) -> None:
    service, state = _context_service(
        tmp_path,
        config=ContextConfig(
            default_context_window_tokens=1024,
            compact_at_percent=0.2,
            target_percent=0.1,
            recent_tail_messages=1,
            model_assisted=False,
        ),
    )
    messages = (
        UserMessage(content="Need to implement compaction. " + "x" * 800),
        ToolResultMessage(
            call_id="call-1",
            name="filesystem.read",
            content='{"path":"src/colossus/application/context.py","content":"large"}',
        ),
        UserMessage(content="Continue from here."),
    )
    await state.append_message("session-1", "run-1", messages[0])
    await state.append_message("session-1", "run-1", messages[1])
    await state.append_message("session-1", "run-1", messages[2])

    result = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=messages,
    )

    snapshots = await state.list_context_snapshots("session-1")
    raw_messages = await state.list_messages("session-1")
    assert result.compacted is True
    assert result.snapshot_id == "snapshot-1"
    assert isinstance(result.messages[0], UserMessage)
    assert "Colossus context snapshot" in result.messages[0].content
    assert "Continue from here." in result.messages[0].content
    assert snapshots[0].files_touched == ("src/colossus/application/context.py",)
    assert raw_messages == messages


@pytest.mark.asyncio
async def test_context_reuses_existing_snapshot_with_new_tail(tmp_path: Path) -> None:
    service, state = _context_service(tmp_path)
    snapshot = ContextSnapshot(
        id="snapshot-1",
        session_id="session-1",
        source_message_range=(1, 2),
        summary="Existing compacted summary.",
    )
    await state.save_context_snapshot(snapshot)
    messages = (
        UserMessage(content="old 1"),
        AssistantMessage(content="old 2"),
        UserMessage(content="new tail"),
    )

    result = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=messages,
    )

    assert result.compacted is True
    assert result.snapshot_id == "snapshot-1"
    assert len(result.messages) == 1
    assert isinstance(result.messages[0], UserMessage)
    assert "new tail" in result.messages[0].content


@pytest.mark.asyncio
async def test_model_assisted_summary_success_and_failure_paths(tmp_path: Path) -> None:
    successful, success_state = _context_service(
        tmp_path / "success",
        config=ContextConfig(model_assisted=True),
    )
    failed, failed_state = _context_service(
        tmp_path / "failed",
        config=ContextConfig(model_assisted=True),
    )
    messages = (UserMessage(content="Summarize this important requirement."),)
    await success_state.append_message("session-1", "run-1", messages[0])
    await failed_state.append_message("session-1", "run-1", messages[0])

    assisted = await successful.compact_session(
        session_id="session-1",
        model="model-a",
        provider=CapturingProvider("assisted summary"),
    )
    fallback = await failed.compact_session(
        session_id="session-1",
        model="model-a",
        provider=CapturingProvider(fail=True),
    )

    assert messages[0].content
    assert assisted.strategy == "hybrid-model"
    assert assisted.summary == "assisted summary"
    assert fallback.strategy == "deterministic"


@pytest.mark.asyncio
async def test_orchestrator_sends_snapshot_plus_recent_tail(tmp_path: Path) -> None:
    provider = CapturingProvider()
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    audit = JsonlAuditSink(tmp_path / "audit.jsonl")
    context_service = ContextService(
        state,
        audit,
        config=ContextConfig(
            default_context_window_tokens=1024,
            compact_at_percent=0.2,
            target_percent=0.1,
            recent_tail_messages=1,
            model_assisted=False,
        ),
        model_context_windows={"model-a": 1024},
        snapshot_id_factory=lambda: "snapshot-1",
    )
    await state.append_message("session-1", "run-0", UserMessage(content="x" * 1000))
    registry = InMemoryToolRegistry(())
    orchestrator = AgentOrchestrator(
        provider=provider,
        tool_registry=registry,
        tool_executor=FunctionToolExecutor({}, registry),
        policy_engine=DefaultPolicyEngine(),
        approval_handler=AllowAllApprovalHandler(),
        state_store=state,
        audit_sink=audit,
        context_service=context_service,
        run_id_factory=lambda: "run-1",
    )

    result = await orchestrator.run(
        AgentRunRequest(prompt="new prompt", agent=default_agent("model-a"), session_id="session-1")
    )

    assert result.final_output == "done"
    assert isinstance(provider.requests[0].messages[0], UserMessage)
    assert "new prompt" in provider.requests[0].messages[0].content
    assert "context.compacted" in (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
