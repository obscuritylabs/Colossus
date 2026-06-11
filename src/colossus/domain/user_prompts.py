"""Structured user prompt domain models."""

from pydantic import BaseModel, ConfigDict


class UserPromptChoice(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    label: str
    description: str = ""


class UserPromptAnswer(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    answer: str
    choice_id: str | None = None
