"""Deep research domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

ResearchDepth = Literal["quick", "standard", "deep"]
ResearchSourceKind = Literal["repo", "web", "mcp"]
ResearchRunStatus = Literal["running", "completed", "failed"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class ResearchSourceDraft(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: ResearchSourceKind
    title: str
    uri: str = ""
    content: str = ""
    query: str = ""
    metadata: dict[str, str] = Field(default_factory=dict)


class ResearchSource(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    run_id: str
    label: str
    kind: ResearchSourceKind
    title: str
    uri: str = ""
    content: str = ""
    query: str = ""
    metadata: dict[str, str] = Field(default_factory=dict)
    created_at: str = Field(default_factory=utc_now_iso)


class ResearchClaim(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    run_id: str
    text: str
    source_labels: tuple[str, ...] = Field(default_factory=tuple)
    created_at: str = Field(default_factory=utc_now_iso)


class ResearchRun(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    question: str
    depth: ResearchDepth = "standard"
    source_kinds: tuple[ResearchSourceKind, ...] = ("repo", "web", "mcp")
    status: ResearchRunStatus = "running"
    report: str = ""
    warnings: tuple[str, ...] = Field(default_factory=tuple)
    created_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)
    completed_at: str | None = None
