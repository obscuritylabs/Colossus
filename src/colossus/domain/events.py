"""Typed event stream emitted by providers and the orchestrator."""

from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field


class ModelDeltaEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["model.delta"] = "model.delta"
    text: str


class ReasoningSummaryEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["reasoning.summary"] = "reasoning.summary"
    summary: str
    provider_format: str | None = None
    detail_id: str | None = None


class ModelRequestPreparedEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["model.request.prepared"] = "model.request.prepared"
    turn: int
    model: str
    instructions: str
    messages: tuple[dict[str, object], ...] = Field(default_factory=tuple)
    tools: tuple[dict[str, object], ...] = Field(default_factory=tuple)


class ToolCallRequestedEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["tool.call.requested"] = "tool.call.requested"
    call_id: str
    name: str
    arguments: dict[str, object] = Field(default_factory=dict)


class ApprovalRequestedEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["approval.requested"] = "approval.requested"
    call_id: str
    reason: str


class ApprovalAutoGrantedEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["approval.auto_granted"] = "approval.auto_granted"
    call_id: str
    reason: str


class RiskAssessmentEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["risk.assessment"] = "risk.assessment"
    call_id: str
    tool: str
    risk_level: Literal["low", "medium", "high"]
    summary: str
    concerns: tuple[str, ...] = Field(default_factory=tuple)
    recommended_decision: Literal["allow", "requires_approval", "deny"]
    model_role: str
    profile_name: str


class ToolCallCompletedEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["tool.call.completed"] = "tool.call.completed"
    call_id: str
    name: str
    output: str
    exit_code: int = 0


class HandoffEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["handoff"] = "handoff"
    from_agent: str
    to_agent: str
    reason: str | None = None


class SubagentStatusEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["subagent.status"] = "subagent.status"
    job_id: str
    status: Literal["queued", "running", "completed", "failed", "cancelled", "interrupted"]
    role: str
    task: str
    message: str = ""


class ResearchStatusEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["research.status"] = "research.status"
    research_id: str
    status: Literal["running", "completed", "failed"]
    phase: str
    message: str = ""
    sources_collected: int = 0


class FinalOutputEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["final.output"] = "final.output"
    text: str


class ErrorEvent(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: Literal["error"] = "error"
    message: str
    recoverable: bool = False


RunEvent = Annotated[
    ModelDeltaEvent
    | ReasoningSummaryEvent
    | ModelRequestPreparedEvent
    | ToolCallRequestedEvent
    | ApprovalRequestedEvent
    | ApprovalAutoGrantedEvent
    | RiskAssessmentEvent
    | ToolCallCompletedEvent
    | HandoffEvent
    | SubagentStatusEvent
    | ResearchStatusEvent
    | FinalOutputEvent
    | ErrorEvent,
    Field(discriminator="type"),
]
