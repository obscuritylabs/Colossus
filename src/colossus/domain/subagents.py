"""Durable subagent job domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

SubagentStatus = Literal[
    "queued",
    "running",
    "completed",
    "failed",
    "cancelled",
    "interrupted",
]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class SubagentJob(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    parent_run_id: str
    parent_call_id: str
    task: str
    role: str = "subagent_default"
    status: SubagentStatus = "queued"
    child_session_id: str
    child_run_id: str | None = None
    final_output: str = ""
    error: str = ""
    created_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)
    started_at: str | None = None
    completed_at: str | None = None
