from collections.abc import AsyncIterator
from pathlib import Path

import pytest
from pydantic import ValidationError

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.approvals import AllowAllApprovalHandler
from colossus.application.context import ContextService
from colossus.application.defaults import default_agent
from colossus.application.memories import MemoryService
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.context import ContextConfig, ContextSnapshot
from colossus.domain.decisions import KeyDecision
from colossus.domain.events import ContextPreparedEvent, FinalOutputEvent, ModelDeltaEvent, RunEvent
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
    memory_service: MemoryService | None = None,
    repo_root: str | None = None,
) -> tuple[ContextService, SQLiteStateStore]:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ContextService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        config=config,
        model_context_windows={"model-a": 1024},
        snapshot_id_factory=lambda: "snapshot-1",
        memory_service=memory_service,
        repo_root=repo_root,
    )
    return service, state


def test_context_config_validates_percentages() -> None:
    assert ContextConfig().compact_at_percent == 0.70
    assert ContextConfig().tool_schema_budget_percent == 0.02

    with pytest.raises(ValidationError):
        ContextConfig(compact_at_percent=0.4, target_percent=0.5)
    with pytest.raises(ValidationError):
        ContextConfig(tool_schema_budget_percent=1.0)


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
async def test_context_reuses_snapshot_without_model_call_for_small_stale_tail(
    tmp_path: Path,
) -> None:
    ids = iter(("snapshot-1", "snapshot-2"))
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ContextService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        config=ContextConfig(
            default_context_window_tokens=8192,
            compact_at_percent=0.7,
            target_percent=0.45,
            recent_tail_messages=1,
            model_assisted=True,
        ),
        model_context_windows={"model-a": 8192},
        snapshot_id_factory=lambda: next(ids),
    )
    provider = CapturingProvider("assisted summary")
    messages = (
        UserMessage(content="Important project history. " + "x" * 18_000),
        AssistantMessage(content="Large implementation notes. " + "y" * 8_000),
        UserMessage(content="Continue from here."),
    )

    first = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=messages,
        provider=provider,
    )
    second = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=(
            *messages,
            ToolResultMessage(
                call_id="call-1",
                name="filesystem.read",
                content='{"path":"src/main.rs","bytes":128}',
            ),
        ),
        provider=provider,
    )

    snapshots = await state.list_context_snapshots("session-1")
    assert first.snapshot_id == "snapshot-1"
    assert first.snapshot_created is True
    assert second.snapshot_id == "snapshot-1"
    assert second.snapshot_created is False
    assert len(provider.requests) == 1
    assert len(snapshots) == 1


@pytest.mark.asyncio
async def test_context_recompacts_when_stale_tail_exceeds_target(
    tmp_path: Path,
) -> None:
    ids = iter(("snapshot-1", "snapshot-2"))
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ContextService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        config=ContextConfig(
            default_context_window_tokens=2048,
            compact_at_percent=0.5,
            target_percent=0.2,
            recent_tail_messages=1,
            model_assisted=True,
        ),
        model_context_windows={"model-a": 2048},
        snapshot_id_factory=lambda: next(ids),
    )
    provider = CapturingProvider("assisted summary")
    messages = (
        UserMessage(content="Important project history. " + "x" * 8_000),
        AssistantMessage(content="Large implementation notes. " + "y" * 4_000),
        UserMessage(content="Continue from here."),
    )
    await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=messages,
        provider=provider,
    )

    result = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=(
            *messages,
            AssistantMessage(content="Large new tool output. " + "z" * 4_000),
            UserMessage(content="Continue again."),
        ),
        provider=provider,
    )

    snapshots = await state.list_context_snapshots("session-1")
    assert result.snapshot_id == "snapshot-2"
    assert result.snapshot_created is True
    assert len(provider.requests) == 2
    assert len(snapshots) == 2
    assert snapshots[0].source_message_range == (1, 4)


@pytest.mark.asyncio
async def test_context_injects_active_key_decisions_before_snapshot(tmp_path: Path) -> None:
    service, state = _context_service(tmp_path)
    snapshot = ContextSnapshot(
        id="snapshot-1",
        session_id="session-1",
        source_message_range=(1, 2),
        summary="Existing compacted summary.",
    )
    await state.save_context_snapshot(snapshot)
    await state.save_decision(
        KeyDecision(
            id="kd_1",
            session_id="session-1",
            source="agent",
            priority="critical",
            title="Durable commitments",
            decision="Key decisions are durable commitments, not memories.",
            intent="Keep commitments stronger than memory context.",
            applies_when="Preparing model context.",
        )
    )
    await state.save_decision(
        KeyDecision(
            id="kd_2",
            session_id="session-1",
            source="agent",
            status="archived",
            priority="high",
            title="Archived",
            decision="Archived decisions should not steer future turns.",
        )
    )
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

    assert isinstance(result.messages[0], UserMessage)
    content = result.messages[0].content
    assert content.index("[Binding active key decisions]") < content.index(
        "[Colossus context snapshot]"
    )
    assert (
        "CRITICAL kd_1 (Durable commitments): "
        "Key decisions are durable commitments, not memories."
    ) in content
    assert "applies_when: Preparing model context." in content
    assert "intent: Keep commitments stronger than memory context." in content
    assert "kd_2" not in content
    assert "new tail" in content


@pytest.mark.asyncio
async def test_context_injects_relevant_memories_after_decisions_before_snapshot(
    tmp_path: Path,
) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    memory_service = MemoryService(state, JsonlAuditSink(tmp_path / "audit.jsonl"), state)
    service = ContextService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        model_context_windows={"model-a": 1024},
        snapshot_id_factory=lambda: "snapshot-1",
        memory_service=memory_service,
        repo_root="/repo",
    )
    snapshot = ContextSnapshot(
        id="snapshot-1",
        session_id="session-1",
        source_message_range=(1, 2),
        summary="Existing compacted summary.",
    )
    await state.save_context_snapshot(snapshot)
    await state.save_decision(
        KeyDecision(
            id="kd_1",
            session_id="session-1",
            source="agent",
            priority="critical",
            title="Commitment",
            decision="Run final gates before completion.",
        )
    )
    await memory_service.create_memory(
        memory_id="mem_repo",
        scope="repo",
        kind="preference",
        text="Run pytest, ruff, and mypy before declaring completion.",
        source="user",
        repo_root="/repo",
    )
    await memory_service.create_memory(
        memory_id="mem_global",
        scope="global",
        kind="capability",
        text="Global note about browser automation.",
        source="agent",
    )
    messages = (
        UserMessage(content="old 1"),
        AssistantMessage(content="old 2"),
        UserMessage(content="Should I run pytest and ruff now?"),
    )

    result = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=messages,
    )

    content = result.messages[0].content
    assert content.index("[Binding active key decisions]") < content.index("[Relevant memories]")
    assert content.index("[Relevant memories]") < content.index("[Colossus context snapshot]")
    assert "REPO/PREFERENCE mem_repo" in content
    assert "mem_global" not in content


@pytest.mark.asyncio
async def test_context_injects_active_key_decisions_without_compaction(tmp_path: Path) -> None:
    service, state = _context_service(tmp_path)
    await state.save_decision(
        KeyDecision(
            id="kd_1",
            session_id="session-1",
            source="user",
            priority="high",
            title="Remember",
            decision="Always preserve active key decisions.",
        )
    )

    result = await service.prepare_messages(
        session_id="session-1",
        model="model-a",
        instructions="",
        messages=(UserMessage(content="short prompt"),),
    )

    assert isinstance(result.messages[0], UserMessage)
    assert "[Binding active key decisions]" in result.messages[0].content
    assert result.messages[1].content == "short prompt"


@pytest.mark.asyncio
async def test_context_status_reports_effective_snapshot_size_and_raw_history(
    tmp_path: Path,
) -> None:
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
        UserMessage(content="Need compaction. " + "x" * 1200),
        AssistantMessage(content="Large reply. " + "y" * 1200),
        UserMessage(content="tail"),
    )
    for message in messages:
        await state.append_message("session-1", "run-1", message)

    await service.compact_session(session_id="session-1", model="model-a")

    status = await service.status("session-1", "model-a")

    assert status.compacted is True
    assert status.latest_snapshot_id == "snapshot-1"
    assert status.raw_token_estimate is not None
    assert status.raw_token_estimate > status.token_estimate
    assert status.raw_token_estimate > status.threshold_tokens
    assert status.token_estimate < status.raw_token_estimate


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
    observed: list[RunEvent] = []
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
        event_observer=observed.append,
    )

    result = await orchestrator.run(
        AgentRunRequest(prompt="new prompt", agent=default_agent("model-a"), session_id="session-1")
    )

    assert result.final_output == "done"
    context_event = next(event for event in observed if isinstance(event, ContextPreparedEvent))
    assert context_event.compacted is True
    assert context_event.snapshot_id == "snapshot-1"
    assert context_event.snapshot_created is True
    assert context_event.original_token_estimate > context_event.token_estimate
    assert isinstance(provider.requests[0].messages[0], UserMessage)
    assert "new prompt" in provider.requests[0].messages[0].content
    assert "context.compacted" in (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
