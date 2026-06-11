"""Session task domain models."""

from typing import Literal

from pydantic import BaseModel, ConfigDict

TaskStatus = Literal["pending", "in_progress", "completed", "blocked", "cancelled"]


class Task(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    title: str
    description: str = ""
    status: TaskStatus = "pending"
    created_at: str
    updated_at: str
