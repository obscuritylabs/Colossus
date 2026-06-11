"""Provider metadata, readiness, and model catalog objects."""

from typing import Literal

from pydantic import BaseModel, ConfigDict


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
