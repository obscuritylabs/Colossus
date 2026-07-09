"""Configuration loading and provider selection."""

import json
import os
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

from colossus.adapters.echo_provider import EchoModelProvider
from colossus.adapters.openai_compat import LocalOpenAIChatProvider, OpenAIResponsesProvider
from colossus.domain.agents import DEFAULT_AGENT_MAX_TURNS, MAX_AGENT_MAX_TURNS
from colossus.domain.context import ContextConfig
from colossus.domain.models import (
    ModelProfile,
    ModelRole,
    ModelRoutingConfig,
    ProviderKind,
)
from colossus.domain.research import ResearchDepth, ResearchSourceKind
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.ports.model_provider import ModelProvider

DEFAULT_MODEL_ROLES: tuple[ModelRole, ...] = (
    "primary",
    "risk_evaluator",
    "context_summarizer",
    "subagent_default",
    "research_planner",
    "research_worker",
    "research_synthesizer",
)


class ProviderConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: ProviderKind = "echo"
    model: str = "default"
    base_url: str | None = None
    api_key_env: str | None = None
    ca_bundle: Path | None = None
    model_context_windows: dict[str, int] = Field(default_factory=dict)


class SubagentConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    max_concurrent: int = Field(default=10, ge=1)


class AgentConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    max_turns: int = Field(default=DEFAULT_AGENT_MAX_TURNS, ge=1, le=MAX_AGENT_MAX_TURNS)


class MemoryIndexConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: Literal["sqlite_fts"] = "sqlite_fts"


class MemoryConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    index: MemoryIndexConfig = Field(default_factory=MemoryIndexConfig)


class HttpConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    ca_bundle: Path | None = None
    client_cert: Path | None = None
    client_key: Path | None = None
    client_key_password_env: str | None = None
    proxy_url: str | None = None
    proxy_url_env: str | None = None
    trust_env: bool = True

    @model_validator(mode="after")
    def _validate_client_cert_pair(self) -> "HttpConfig":
        _validate_client_cert_config(
            client_cert=self.client_cert,
            client_key=self.client_key,
            client_key_password_env=self.client_key_password_env,
        )
        return self


class SearchConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: Literal["disabled", "duckduckgo", "searxng"] = "disabled"
    endpoint: str = "https://duckduckgo.com/html/"
    api_key_env: str | None = None
    auth_header: str = "Authorization"
    auth_scheme: Literal["bearer", "raw"] = "bearer"
    user_agent: str = "colossus-agent/0.1"


class McpResearchToolConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    tool: str
    arguments: dict[str, object] = Field(default_factory=dict)
    title: str = ""


class McpServerConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    command: str
    args: tuple[str, ...] = Field(default_factory=tuple)
    env: dict[str, str] = Field(default_factory=dict)
    allowed_tools: tuple[str, ...] = Field(default_factory=tuple)
    research_tools: tuple[McpResearchToolConfig, ...] = Field(default_factory=tuple)


class McpConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    servers: dict[str, McpServerConfig] = Field(default_factory=dict)


class ResearchConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    default_depth: ResearchDepth = "standard"
    max_sources: int = Field(default=20, ge=1, le=100)
    max_workers: int = Field(default=4, ge=1, le=16)
    sources: tuple[ResearchSourceKind, ...] = ("repo", "web", "mcp")
    search: SearchConfig = Field(default_factory=SearchConfig)
    mcp: McpConfig = Field(default_factory=McpConfig)


class ColossusConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    provider: ProviderConfig = Field(default_factory=ProviderConfig)
    models: ModelRoutingConfig = Field(default_factory=ModelRoutingConfig)
    context: ContextConfig = Field(default_factory=ContextConfig)
    agent: AgentConfig = Field(default_factory=AgentConfig)
    subagents: SubagentConfig = Field(default_factory=SubagentConfig)
    memory: MemoryConfig = Field(default_factory=MemoryConfig)
    http: HttpConfig = Field(default_factory=HttpConfig)
    research: ResearchConfig = Field(default_factory=ResearchConfig)
    allow_user_skill_overrides: bool = False


class ProviderOverrides(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: ProviderKind | None = None
    model: str | None = None
    context_window_tokens: int | None = Field(default=None, gt=0)
    base_url: str | None = None
    api_key: str | None = None
    api_key_env: str | None = None
    ca_bundle: Path | None = None


class HttpOverrides(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    ca_bundle: Path | None = None
    client_cert: Path | None = None
    client_key: Path | None = None
    client_key_password_env: str | None = None
    proxy_url: str | None = None
    proxy_url_env: str | None = None
    trust_env: bool | None = None


def default_config() -> ColossusConfig:
    return ColossusConfig()


def load_config(path: Path) -> ColossusConfig:
    if not path.exists():
        return default_config()
    return ColossusConfig.model_validate_json(path.read_text(encoding="utf-8"))


def write_default_config(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(default_config().model_dump_json(indent=2), encoding="utf-8")


def http_client_config_from_config(
    config: ColossusConfig,
    overrides: HttpOverrides | None = None,
) -> HttpClientConfig:
    overrides = overrides or HttpOverrides()
    http = config.http
    client_key_password_env = (
        overrides.client_key_password_env or http.client_key_password_env
    )
    proxy_url_env = overrides.proxy_url_env or http.proxy_url_env
    ca_bundle = overrides.ca_bundle or http.ca_bundle
    client_cert = overrides.client_cert or http.client_cert
    client_key = overrides.client_key or http.client_key
    _validate_client_cert_config(
        client_cert=client_cert,
        client_key=client_key,
        client_key_password_env=client_key_password_env,
    )
    return HttpClientConfig(
        ca_bundle=ca_bundle,
        client_cert=client_cert,
        client_key=client_key,
        client_key_password=_env_value(client_key_password_env),
        proxy_url=(
            overrides.proxy_url
            or _env_value(proxy_url_env)
            or http.proxy_url
        ),
        trust_env=http.trust_env if overrides.trust_env is None else overrides.trust_env,
    )


def provider_from_config(
    config: ColossusConfig,
    overrides: ProviderOverrides | None = None,
    *,
    require_credentials: bool = True,
    http_client_config: HttpClientConfig | None = None,
) -> ModelProvider:
    overrides = overrides or ProviderOverrides()
    provider = config.provider
    kind = overrides.kind or provider.kind
    base_url = overrides.base_url or provider.base_url
    api_key_env = overrides.api_key_env or provider.api_key_env or "OPENAI_API_KEY"
    api_key = overrides.api_key or os.environ.get(api_key_env, "")
    ca_bundle = overrides.ca_bundle or provider.ca_bundle
    resolved_http = http_client_config or http_client_config_from_config(config)

    if kind == "echo":
        return EchoModelProvider()
    if kind == "openai_responses":
        if not api_key and require_credentials:
            raise ValueError(
                "OpenAI Responses provider requires an API key environment variable."
            )
        return OpenAIResponsesProvider(
            api_key=api_key,
            base_url=base_url or "https://api.openai.com/v1",
            ca_bundle=ca_bundle,
            http_client_config=resolved_http,
        )
    if kind == "local_openai_chat":
        return LocalOpenAIChatProvider(
            base_url=base_url or "http://localhost:8000/v1",
            api_key=api_key or "local",
            ca_bundle=ca_bundle,
            http_client_config=resolved_http,
        )
    raise ValueError(f"Unsupported provider: {kind}")


def effective_model_routing(
    config: ColossusConfig,
    overrides: ProviderOverrides | None = None,
) -> ModelRoutingConfig:
    """Return model profiles/roles with legacy provider config mapped to primary."""
    overrides = overrides or ProviderOverrides()
    profiles = dict(config.models.profiles)
    roles = dict(config.models.roles)
    primary_profile_name = roles.get("primary", "primary")

    if not profiles or primary_profile_name not in profiles:
        profiles[primary_profile_name] = _profile_from_provider_config(config.provider)

    if _has_primary_override(overrides):
        profiles[primary_profile_name] = _profile_with_overrides(
            profiles[primary_profile_name],
            overrides,
        )

    roles["primary"] = primary_profile_name
    for role in DEFAULT_MODEL_ROLES:
        roles.setdefault(role, primary_profile_name)

    for configured_role, profile_name in roles.items():
        if profile_name not in profiles:
            raise ValueError(
                f"Model role {configured_role!r} references unknown profile {profile_name!r}."
            )
    return ModelRoutingConfig(profiles=profiles, roles=roles)


def provider_from_profile(
    profile: ModelProfile,
    *,
    api_key: str | None = None,
    require_credentials: bool = True,
    http_client_config: HttpClientConfig | None = None,
) -> ModelProvider:
    api_key_env = profile.api_key_env or "OPENAI_API_KEY"
    resolved_api_key = api_key or os.environ.get(api_key_env, "")
    ca_bundle = Path(profile.ca_bundle).expanduser() if profile.ca_bundle else None
    resolved_http = http_client_config or HttpClientConfig()

    if profile.provider == "echo":
        return EchoModelProvider()
    if profile.provider == "openai_responses":
        if not resolved_api_key and require_credentials:
            raise ValueError(
                "OpenAI Responses provider requires an API key environment variable."
            )
        return OpenAIResponsesProvider(
            api_key=resolved_api_key,
            base_url=profile.base_url or "https://api.openai.com/v1",
            ca_bundle=ca_bundle,
            http_client_config=resolved_http,
        )
    if profile.provider == "local_openai_chat":
        return LocalOpenAIChatProvider(
            base_url=profile.base_url or "http://localhost:8000/v1",
            api_key=resolved_api_key or "local",
            ca_bundle=ca_bundle,
            http_client_config=resolved_http,
        )
    raise ValueError(f"Unsupported provider: {profile.provider}")


def model_context_windows_from_routing(routing: ModelRoutingConfig) -> dict[str, int]:
    windows: dict[str, int] = {}
    for profile in routing.profiles.values():
        if profile.context_window_tokens is not None:
            windows[profile.model] = profile.context_window_tokens
    return windows


def as_pretty_json(config: ColossusConfig) -> str:
    return json.dumps(config.model_dump(mode="json"), indent=2)


def _profile_from_provider_config(provider: ProviderConfig) -> ModelProfile:
    return ModelProfile(
        provider=provider.kind,
        model=provider.model,
        base_url=provider.base_url,
        api_key_env=provider.api_key_env,
        ca_bundle=str(provider.ca_bundle) if provider.ca_bundle is not None else None,
        context_window_tokens=provider.model_context_windows.get(provider.model),
    )


def _has_primary_override(overrides: ProviderOverrides) -> bool:
    return any(
        value is not None
        for value in (
            overrides.kind,
            overrides.model,
            overrides.context_window_tokens,
            overrides.base_url,
            overrides.api_key_env,
            overrides.ca_bundle,
            overrides.api_key,
        )
    )


def _profile_with_overrides(profile: ModelProfile, overrides: ProviderOverrides) -> ModelProfile:
    return profile.model_copy(
        update={
            "provider": overrides.kind or profile.provider,
            "model": overrides.model or profile.model,
            "context_window_tokens": overrides.context_window_tokens
            or profile.context_window_tokens,
            "base_url": overrides.base_url or profile.base_url,
            "api_key_env": overrides.api_key_env or profile.api_key_env,
            "ca_bundle": str(overrides.ca_bundle) if overrides.ca_bundle else profile.ca_bundle,
        }
    )


def _validate_client_cert_config(
    *,
    client_cert: Path | None,
    client_key: Path | None,
    client_key_password_env: str | None,
) -> None:
    if client_key is not None and client_cert is None:
        raise ValueError("client_key requires client_cert.")
    if client_key_password_env is not None and client_key is None:
        raise ValueError("client_key_password_env requires client_key.")


def _env_value(name: str | None) -> str | None:
    if name is None:
        return None
    value = os.environ.get(name)
    return value or None
