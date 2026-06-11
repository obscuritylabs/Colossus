"""Session and message persistence models."""

from datetime import UTC, datetime

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.messages import Message


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class Session(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    title: str | None = None
    created_at: str = Field(default_factory=utc_now_iso)


class MessageRecord(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    session_id: str
    run_id: str
    sequence: int
    message: Message
    created_at: str = Field(default_factory=utc_now_iso)
