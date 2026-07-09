"""Operational telemetry models derived from persisted run events."""

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.events import RunEvent


class RunEventRecord(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    sequence: int
    run_id: str
    event_type: str
    created_at: str
    event: RunEvent
    session_id: str | None = None


class RunTelemetrySummary(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    run_id: str
    session_id: str | None = None
    started_at: str
    last_event_at: str
    duration_seconds: float = 0.0
    events: int = 0
    event_types: dict[str, int] = Field(default_factory=dict)
    model_output_chars: int = 0
    tool_calls: int = 0
    tool_errors: int = 0
    approval_requests: int = 0
    auto_approvals: int = 0
    risk_assessments: int = 0
    research_events: int = 0
    subagent_events: int = 0
    context_compactions: int = 0
    error_events: int = 0
    final_outputs: int = 0


class RunTelemetryDetail(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    summary: RunTelemetrySummary
    records: tuple[RunEventRecord, ...] = Field(default_factory=tuple)


class TelemetryMetrics(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    run_count: int = 0
    event_count: int = 0
    average_duration_seconds: float = 0.0
    max_duration_seconds: float = 0.0
    model_output_chars: int = 0
    tool_calls: int = 0
    tool_errors: int = 0
    approval_requests: int = 0
    auto_approvals: int = 0
    risk_assessments: int = 0
    research_events: int = 0
    subagent_events: int = 0
    context_compactions: int = 0
    error_events: int = 0
    final_outputs: int = 0
    event_types: dict[str, int] = Field(default_factory=dict)
