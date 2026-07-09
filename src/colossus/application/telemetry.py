"""Derived observability queries for agent runs."""

from collections import Counter
from datetime import UTC, datetime

from colossus.domain.errors import ColossusError
from colossus.domain.events import (
    ContextPreparedEvent,
    ErrorEvent,
    FinalOutputEvent,
    ModelDeltaEvent,
    ResearchProgressEvent,
    ResearchStatusEvent,
    RiskAssessmentEvent,
    SubagentStatusEvent,
    ToolCallCompletedEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.telemetry import (
    RunEventRecord,
    RunTelemetryDetail,
    RunTelemetrySummary,
    TelemetryMetrics,
)
from colossus.ports.state import StateStore


class TelemetryService:
    """Summarize persisted event metadata without exposing raw model internals."""

    def __init__(self, state_store: StateStore) -> None:
        self._state_store = state_store

    async def list_runs(
        self,
        *,
        session_id: str | None = None,
        limit: int = 20,
    ) -> tuple[RunTelemetrySummary, ...]:
        records = await self._state_store.list_run_event_records(
            session_id=session_id,
            limit=limit,
        )
        grouped: dict[str, list[RunEventRecord]] = {}
        for record in records:
            grouped.setdefault(record.run_id, []).append(record)
        return tuple(_summarize_run(tuple(items)) for items in grouped.values())

    async def get_run(self, run_id: str) -> RunTelemetryDetail:
        records = await self._state_store.list_run_event_records(
            run_id=run_id,
            limit=10_000,
        )
        if not records:
            records = await self._records_for_unique_prefix(run_id)
        if not records:
            raise ColossusError(f"No telemetry events found for run: {run_id}")
        return RunTelemetryDetail(summary=_summarize_run(records), records=records)

    async def metrics(
        self,
        *,
        session_id: str | None = None,
        limit: int = 100,
    ) -> TelemetryMetrics:
        summaries = await self.list_runs(session_id=session_id, limit=limit)
        if not summaries:
            return TelemetryMetrics()
        event_types: Counter[str] = Counter()
        durations = [summary.duration_seconds for summary in summaries]
        for summary in summaries:
            event_types.update(summary.event_types)
        return TelemetryMetrics(
            run_count=len(summaries),
            event_count=sum(summary.events for summary in summaries),
            average_duration_seconds=sum(durations) / len(durations),
            max_duration_seconds=max(durations),
            model_output_chars=sum(summary.model_output_chars for summary in summaries),
            tool_calls=sum(summary.tool_calls for summary in summaries),
            tool_errors=sum(summary.tool_errors for summary in summaries),
            approval_requests=sum(summary.approval_requests for summary in summaries),
            auto_approvals=sum(summary.auto_approvals for summary in summaries),
            risk_assessments=sum(summary.risk_assessments for summary in summaries),
            research_events=sum(summary.research_events for summary in summaries),
            subagent_events=sum(summary.subagent_events for summary in summaries),
            context_compactions=sum(summary.context_compactions for summary in summaries),
            error_events=sum(summary.error_events for summary in summaries),
            final_outputs=sum(summary.final_outputs for summary in summaries),
            event_types=dict(sorted(event_types.items())),
        )

    async def _records_for_unique_prefix(self, run_id_prefix: str) -> tuple[RunEventRecord, ...]:
        if len(run_id_prefix) < 4:
            return ()
        recent = await self._state_store.list_run_event_records(limit=500)
        matches = sorted(
            {record.run_id for record in recent if record.run_id.startswith(run_id_prefix)}
        )
        if len(matches) > 1:
            raise ColossusError(f"Ambiguous telemetry run id prefix: {run_id_prefix}")
        if not matches:
            return ()
        return await self._state_store.list_run_event_records(
            run_id=matches[0],
            limit=10_000,
        )


def _summarize_run(records: tuple[RunEventRecord, ...]) -> RunTelemetrySummary:
    if not records:
        raise ColossusError("Cannot summarize an empty telemetry run.")
    event_types: Counter[str] = Counter(record.event_type for record in records)
    started_at = records[0].created_at
    last_event_at = records[-1].created_at
    model_output_chars = 0
    tool_calls = 0
    tool_errors = 0
    approval_requests = 0
    auto_approvals = 0
    risk_assessments = 0
    research_events = 0
    subagent_events = 0
    context_compactions = 0
    error_events = 0
    final_outputs = 0

    for record in records:
        event = record.event
        if isinstance(event, ModelDeltaEvent):
            model_output_chars += len(event.text)
        elif isinstance(event, ToolCallRequestedEvent):
            tool_calls += 1
        elif isinstance(event, ToolCallCompletedEvent) and event.exit_code != 0:
            tool_errors += 1
        elif record.event_type == "approval.requested":
            approval_requests += 1
        elif record.event_type == "approval.auto_granted":
            auto_approvals += 1
        elif isinstance(event, RiskAssessmentEvent):
            risk_assessments += 1
        elif isinstance(event, ResearchProgressEvent | ResearchStatusEvent):
            research_events += 1
        elif isinstance(event, SubagentStatusEvent):
            subagent_events += 1
        elif isinstance(event, ContextPreparedEvent) and event.compacted:
            context_compactions += 1
        elif isinstance(event, ErrorEvent):
            error_events += 1
        elif isinstance(event, FinalOutputEvent):
            final_outputs += 1
            model_output_chars += len(event.text)

    return RunTelemetrySummary(
        run_id=records[0].run_id,
        session_id=records[0].session_id,
        started_at=started_at,
        last_event_at=last_event_at,
        duration_seconds=_duration_seconds(started_at, last_event_at),
        events=len(records),
        event_types=dict(sorted(event_types.items())),
        model_output_chars=model_output_chars,
        tool_calls=tool_calls,
        tool_errors=tool_errors,
        approval_requests=approval_requests,
        auto_approvals=auto_approvals,
        risk_assessments=risk_assessments,
        research_events=research_events,
        subagent_events=subagent_events,
        context_compactions=context_compactions,
        error_events=error_events,
        final_outputs=final_outputs,
    )


def _duration_seconds(started_at: str, ended_at: str) -> float:
    started = _parse_timestamp(started_at)
    ended = _parse_timestamp(ended_at)
    if started is None or ended is None:
        return 0.0
    return max(0.0, (ended - started).total_seconds())


def _parse_timestamp(value: str) -> datetime | None:
    normalized = value.strip().replace(" ", "T")
    if normalized.endswith("Z"):
        normalized = normalized[:-1] + "+00:00"
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed
