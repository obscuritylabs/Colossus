"""Model profiles and role routing domain objects."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

ProviderKind = Literal["echo", "openai_responses", "local_openai_chat"]
ModelRole = Literal[
    "primary",
    "risk_evaluator",
    "context_summarizer",
    "subagent_default",
    "research_planner",
    "research_worker",
    "research_synthesizer",
]


class ModelProfile(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    provider: ProviderKind = "echo"
    model: str = "default"
    base_url: str | None = None
    api_key_env: str | None = None
    ca_bundle: str | None = None
    context_window_tokens: int | None = Field(default=None, gt=0)


class ModelRoutingConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    profiles: dict[str, ModelProfile] = Field(default_factory=dict)
    roles: dict[str, str] = Field(default_factory=dict)


class ResolvedModelProfile(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    role: str
    profile_name: str
    provider: ProviderKind
    model: str
    base_url: str | None = None
    api_key_env: str | None = None
    ca_bundle: str | None = None
    context_window_tokens: int | None = None
