"""Plan-mode domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

RunMode = Literal["chat", "plan", "execute"]
PlanStatus = Literal["draft", "approved", "executed"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class PlanStep(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    index: int
    title: str
    detail: str
    requires_mutation: bool = False


class Plan(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    prompt: str
    status: PlanStatus = "draft"
    content: str = ""
    steps: tuple[PlanStep, ...] = Field(default_factory=tuple)
    created_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)

    @property
    def requires_approval(self) -> bool:
        return bool(self.content.strip()) or any(step.requires_mutation for step in self.steps)
