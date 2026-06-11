"""Audit records."""

from datetime import UTC, datetime

from pydantic import BaseModel, ConfigDict, Field


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class AuditRecord(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    seq: int
    prev_hash: str
    ts: str = Field(default_factory=utc_now_iso)
    actor: str
    event: str
    policy_decision: str | None = None
    details: dict[str, object] = Field(default_factory=dict)
    hash: str | None = None
