"""Durable key decision domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

DecisionSource = Literal["user", "agent"]
DecisionStatus = Literal["active", "archived", "superseded"]
DecisionPriority = Literal["critical", "high", "normal"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class KeyDecision(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    goal_id: str | None = None
    plan_id: str | None = None
    source: DecisionSource
    status: DecisionStatus = "active"
    priority: DecisionPriority = "normal"
    title: str
    decision: str
    intent: str = ""
    applies_when: str = ""
    rationale: str = ""
    source_excerpt: str = ""
    supersedes: str | None = None
    created_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)
