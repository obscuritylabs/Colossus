"""Context compaction domain models."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from colossus.domain.messages import Message


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class ContextConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    auto_compaction: bool = True
    default_context_window_tokens: int = 32_768
    compact_at_percent: float = 0.70
    target_percent: float = 0.45
    recent_tail_messages: int = 8
    model_assisted: bool = True

    @field_validator("default_context_window_tokens")
    @classmethod
    def _positive_window(cls, value: int) -> int:
        if value < 1024:
            raise ValueError("default_context_window_tokens must be at least 1024.")
        return value

    @field_validator("compact_at_percent", "target_percent")
    @classmethod
    def _valid_percent(cls, value: float) -> float:
        if value <= 0 or value >= 1:
            raise ValueError("context percentages must be greater than 0 and less than 1.")
        return value

    @field_validator("recent_tail_messages")
    @classmethod
    def _non_negative_tail(cls, value: int) -> int:
        if value < 0:
            raise ValueError("recent_tail_messages must be non-negative.")
        return value

    @model_validator(mode="after")
    def _target_below_threshold(self) -> "ContextConfig":
        if self.target_percent >= self.compact_at_percent:
            raise ValueError("target_percent must be lower than compact_at_percent.")
        return self


class ContextSnapshot(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    session_id: str
    source_message_range: tuple[int, int]
    summary: str
    pinned_facts: tuple[str, ...] = Field(default_factory=tuple)
    open_tasks: tuple[str, ...] = Field(default_factory=tuple)
    files_touched: tuple[str, ...] = Field(default_factory=tuple)
    tool_results: tuple[str, ...] = Field(default_factory=tuple)
    created_at: str = Field(default_factory=utc_now_iso)
    strategy: Literal["deterministic", "hybrid-model"] = "deterministic"


class ContextBuildResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    messages: tuple[Message, ...]
    token_estimate: int
    original_token_estimate: int
    context_window_tokens: int
    threshold_tokens: int
    target_tokens: int
    snapshot_id: str | None = None
    compacted: bool = False


class ContextStatus(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    session_id: str
    model: str
    message_count: int
    token_estimate: int
    context_window_tokens: int
    threshold_tokens: int
    target_tokens: int
    latest_snapshot_id: str | None = None
    auto_compaction: bool = True
