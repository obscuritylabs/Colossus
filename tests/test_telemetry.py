from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.telemetry import TelemetryService
from colossus.domain.events import (
    FinalOutputEvent,
    ModelDeltaEvent,
    ResearchProgressEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.messages import UserMessage


async def test_telemetry_service_summarizes_persisted_run_events(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.db")
    await state.append_message("session-1", "run-1", UserMessage(content="observe this"))
    await state.append_event("run-1", ModelDeltaEvent(text="working"))
    await state.append_event(
        "run-1",
        ToolCallRequestedEvent(call_id="call-1", name="shell.run", arguments={"cmd": "test"}),
    )
    await state.append_event(
        "run-1",
        ToolCallCompletedEvent(
            call_id="call-1",
            name="shell.run",
            output="tool failure body that should only count by metadata",
            exit_code=1,
        ),
    )
    await state.append_event(
        "run-1",
        ResearchProgressEvent(
            research_id="research-1",
            phase="collecting",
            action="repo",
            status="completed",
            sources_collected=2,
        ),
    )
    await state.append_event("run-1", FinalOutputEvent(text="done"))

    service = TelemetryService(state)
    summaries = await service.list_runs(session_id="session-1")
    detail = await service.get_run("run-1")
    metrics = await service.metrics(session_id="session-1")

    assert len(summaries) == 1
    assert summaries[0].run_id == "run-1"
    assert summaries[0].session_id == "session-1"
    assert summaries[0].events == 5
    assert summaries[0].tool_calls == 1
    assert summaries[0].tool_errors == 1
    assert summaries[0].research_events == 1
    assert summaries[0].model_output_chars == len("workingdone")
    assert detail.records[1].event_type == "tool.call.requested"
    assert metrics.run_count == 1
    assert metrics.event_types["tool.call.completed"] == 1
