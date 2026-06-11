"""Configuration loading and provider selection."""

import json
import os
from pathlib import Path

from pydantic import BaseModel, ConfigDict, Field

from colossus.adapters.echo_provider import EchoModelProvider
from colossus.adapters.local_openai_chat import LocalOpenAIChatProvider
from colossus.adapters.openai_responses import OpenAIResponsesProvider
from colossus.domain.context import ContextConfig
from colossus.domain.models import (
    ModelProfile,
    ModelRole,
    ModelRoutingConfig,
    ProviderKind,
)
from colossus.ports.model_provider import ModelProvider

DEFAULT_MODEL_ROLES: tuple[ModelRole, ...] = (
    "primary",
    "risk_evaluator",
    "context_summarizer",
    "subagent_default",
)


class ProviderConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: ProviderKind = "echo"
    model: str = "default"
    base_url: str | None = None
    api_key_env: str | None = None
    ca_bundle: Path | None = None
    model_context_windows: dict[str, int] = Field(default_factory=dict)


class ColossusConfig(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    provider: ProviderConfig = Field(default_factory=ProviderConfig)
    models: ModelRoutingConfig = Field(default_factory=ModelRoutingConfig)
    context: ContextConfig = Field(default_factory=ContextConfig)
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


def default_config() -> ColossusConfig:
    return ColossusConfig()


def load_config(path: Path) -> ColossusConfig:
    if not path.exists():
        return default_config()
    return ColossusConfig.model_validate_json(path.read_text(encoding="utf-8"))


def write_default_config(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(default_config().model_dump_json(indent=2), encoding="utf-8")


def provider_from_config(
    config: ColossusConfig,
    overrides: ProviderOverrides | None = None,
    *,
    require_credentials: bool = True,
) -> ModelProvider:
    overrides = overrides or ProviderOverrides()
    provider = config.provider
    kind = overrides.kind or provider.kind
    base_url = overrides.base_url or provider.base_url
    api_key_env = overrides.api_key_env or provider.api_key_env or "OPENAI_API_KEY"
    api_key = overrides.api_key or os.environ.get(api_key_env, "")
    ca_bundle = overrides.ca_bundle or provider.ca_bundle

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
        )
    if kind == "local_openai_chat":
        return LocalOpenAIChatProvider(
            base_url=base_url or "http://localhost:8000/v1",
            api_key=api_key or "local",
            ca_bundle=ca_bundle,
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
) -> ModelProvider:
    api_key_env = profile.api_key_env or "OPENAI_API_KEY"
    resolved_api_key = api_key or os.environ.get(api_key_env, "")
    ca_bundle = Path(profile.ca_bundle).expanduser() if profile.ca_bundle else None

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
        )
    if profile.provider == "local_openai_chat":
        return LocalOpenAIChatProvider(
            base_url=profile.base_url or "http://localhost:8000/v1",
            api_key=resolved_api_key or "local",
            ca_bundle=ca_bundle,
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
