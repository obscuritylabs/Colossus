"""Risk assessment domain models."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.events import RiskAssessmentEvent

RiskLevel = Literal["low", "medium", "high"]
RiskDecision = Literal["allow", "requires_approval", "deny"]


class RiskAssessment(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    tool: str
    risk_level: RiskLevel
    summary: str
    concerns: tuple[str, ...] = Field(default_factory=tuple)
    recommended_decision: RiskDecision
    model_role: str = "risk_evaluator"
    profile_name: str = "primary"

    def to_event(self, call_id: str) -> RiskAssessmentEvent:
        return RiskAssessmentEvent(
            call_id=call_id,
            tool=self.tool,
            risk_level=self.risk_level,
            summary=self.summary,
            concerns=self.concerns,
            recommended_decision=self.recommended_decision,
            model_role=self.model_role,
            profile_name=self.profile_name,
        )
