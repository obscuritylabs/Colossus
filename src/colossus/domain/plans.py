"""Plan-mode domain models."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

RunMode = Literal["chat", "plan", "execute"]
PlanStatus = Literal["draft", "approved", "executed"]


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
    steps: tuple[PlanStep, ...] = Field(default_factory=tuple)

    @property
    def requires_approval(self) -> bool:
        return any(step.requires_mutation for step in self.steps)
