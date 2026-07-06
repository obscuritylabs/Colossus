"""Goal-mode domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

GoalStatus = Literal["active", "complete", "blocked"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class Goal(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    objective: str
    source_plan_id: str | None = None
    status: GoalStatus = "active"
    summary: str = ""
    blocked_reason: str = ""
    iteration_budget: int | None = None
    iterations_completed: int = 0
    created_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)
