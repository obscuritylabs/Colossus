"""Provider metadata, readiness, and model catalog objects."""

from collections.abc import Iterable
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class ProviderCapability(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    supported: bool
    detail: str | None = None


class ProviderReadinessCheck(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    status: Literal["pass", "fail"]
    detail: str


class ProviderReadiness(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    provider: str
    ready: bool
    checks: tuple[ProviderReadinessCheck, ...]


class ProviderModelInfo(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    id: str
    owner: str | None = None
    created: int | None = None
    context_window_tokens: int | None = Field(default=None, gt=0)
    max_output_tokens: int | None = Field(default=None, gt=0)


def model_context_windows_from_provider_models(
    models: Iterable[ProviderModelInfo],
) -> dict[str, int]:
    """Return discovered model context windows keyed by provider model id."""
    return {
        model.id: model.context_window_tokens
        for model in models
        if model.context_window_tokens is not None
    }
