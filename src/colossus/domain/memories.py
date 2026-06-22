"""Durable memory domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

MemoryScope = Literal["global", "repo", "session"]
MemoryKind = Literal["preference", "project_fact", "episode", "capability", "warning"]
MemoryStatus = Literal["active", "archived", "superseded"]
MemorySource = Literal["user", "agent"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class MemoryItem(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    scope: MemoryScope
    kind: MemoryKind
    status: MemoryStatus = "active"
    source: MemorySource
    confidence: float = Field(default=1.0, ge=0.0, le=1.0)
    text: str
    rationale: str = ""
    repo_root: str | None = None
    session_id: str | None = None
    supersedes: str | None = None
    stale_after: str | None = None
    expires_at: str | None = None
    created_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)
