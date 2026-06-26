from pathlib import Path

import pytest
from pydantic import ValidationError

from colossus.adapters.local_openai_chat import LocalOpenAIChatProvider
from colossus.adapters.openai_responses import OpenAIResponsesProvider
from colossus.domain.models import ModelProfile, ModelRoutingConfig
from colossus.infrastructure.config import (
    ColossusConfig,
    HttpConfig,
    HttpOverrides,
    ProviderConfig,
    ProviderOverrides,
    effective_model_routing,
    http_client_config_from_config,
    provider_from_config,
)


def test_local_provider_uses_configured_ca_bundle(tmp_path: Path) -> None:
    ca_bundle = tmp_path / "ca.pem"
    ca_bundle.write_text("test-ca", encoding="utf-8")
    config = ColossusConfig(
        provider=ProviderConfig(kind="local_openai_chat", ca_bundle=ca_bundle),
    )

    provider = provider_from_config(config)

    assert isinstance(provider, LocalOpenAIChatProvider)
    assert provider.ca_bundle == ca_bundle


def test_cli_ca_bundle_override_wins_for_openai_provider(tmp_path: Path, monkeypatch) -> None:
    configured_ca = tmp_path / "configured.pem"
    override_ca = tmp_path / "override.pem"
    configured_ca.write_text("configured", encoding="utf-8")
    override_ca.write_text("override", encoding="utf-8")
    monkeypatch.setenv("OPENAI_API_KEY", "test-key")
    config = ColossusConfig(
        provider=ProviderConfig(kind="openai_responses", ca_bundle=configured_ca),
    )

    provider = provider_from_config(config, ProviderOverrides(ca_bundle=override_ca))

    assert isinstance(provider, OpenAIResponsesProvider)
    assert provider.ca_bundle == override_ca


def test_global_http_ca_bundle_applies_to_provider_when_provider_ca_is_unset(
    tmp_path: Path,
) -> None:
    ca_bundle = tmp_path / "global-ca.pem"
    ca_bundle.write_text("test-ca", encoding="utf-8")
    config = ColossusConfig(
        provider=ProviderConfig(kind="local_openai_chat"),
        http=HttpConfig(ca_bundle=ca_bundle),
    )

    provider = provider_from_config(config)

    assert isinstance(provider, LocalOpenAIChatProvider)
    assert provider.ca_bundle == ca_bundle


def test_provider_ca_bundle_overrides_global_http_ca_bundle(tmp_path: Path) -> None:
    global_ca = tmp_path / "global-ca.pem"
    provider_ca = tmp_path / "provider-ca.pem"
    global_ca.write_text("global", encoding="utf-8")
    provider_ca.write_text("provider", encoding="utf-8")
    config = ColossusConfig(
        provider=ProviderConfig(kind="local_openai_chat", ca_bundle=provider_ca),
        http=HttpConfig(ca_bundle=global_ca),
    )

    provider = provider_from_config(config)

    assert isinstance(provider, LocalOpenAIChatProvider)
    assert provider.ca_bundle == provider_ca


def test_http_client_config_resolves_proxy_and_client_cert_from_env(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    ca_bundle = tmp_path / "ca.pem"
    client_cert = tmp_path / "client.pem"
    client_key = tmp_path / "client.key"
    ca_bundle.write_text("ca", encoding="utf-8")
    client_cert.write_text("cert", encoding="utf-8")
    client_key.write_text("key", encoding="utf-8")
    monkeypatch.setenv("COLOSSUS_PROXY", "http://proxy.example.test:8080")
    monkeypatch.setenv("COLOSSUS_KEY_PASSWORD", "secret")
    config = ColossusConfig(
        http=HttpConfig(
            ca_bundle=ca_bundle,
            client_cert=client_cert,
            client_key=client_key,
            client_key_password_env="COLOSSUS_KEY_PASSWORD",
            proxy_url_env="COLOSSUS_PROXY",
            trust_env=False,
        )
    )

    http_config = http_client_config_from_config(config)

    assert http_config.ca_bundle == ca_bundle
    assert http_config.cert == (str(client_cert), str(client_key), "secret")
    assert http_config.proxy_url == "http://proxy.example.test:8080"
    assert http_config.trust_env is False
    assert http_config.async_client_kwargs(timeout=1.0)["proxy"] == (
        "http://proxy.example.test:8080"
    )


def test_http_client_config_overrides_merge_with_config(tmp_path: Path) -> None:
    client_cert = tmp_path / "client.pem"
    client_key = tmp_path / "client.key"
    client_cert.write_text("cert", encoding="utf-8")
    client_key.write_text("key", encoding="utf-8")
    config = ColossusConfig(
        http=HttpConfig(client_cert=client_cert, client_key=client_key)
    )

    http_config = http_client_config_from_config(
        config,
        HttpOverrides(proxy_url="http://proxy.example.test:8080", trust_env=False),
    )

    assert http_config.cert == (str(client_cert), str(client_key))
    assert http_config.proxy_url == "http://proxy.example.test:8080"
    assert http_config.trust_env is False


def test_http_config_rejects_partial_client_key_settings(tmp_path: Path) -> None:
    client_key = tmp_path / "client.key"
    client_key.write_text("key", encoding="utf-8")

    with pytest.raises(ValueError, match="client_key requires client_cert"):
        ColossusConfig(http=HttpConfig(client_key=client_key))
    with pytest.raises(ValueError, match="client_key_password_env requires client_key"):
        ColossusConfig(http=HttpConfig(client_key_password_env="COLOSSUS_KEY_PASSWORD"))


def test_provider_override_sets_openai_base_url_and_api_key(tmp_path: Path) -> None:
    config = ColossusConfig(provider=ProviderConfig(kind="echo"))

    provider = provider_from_config(
        config,
        ProviderOverrides(
            kind="openai_responses",
            base_url="https://gateway.example.test/v1",
            api_key="test-key",
        ),
    )

    assert isinstance(provider, OpenAIResponsesProvider)
    assert provider.base_url == "https://gateway.example.test/v1"


def test_config_supports_context_defaults_and_model_windows() -> None:
    config = ColossusConfig(
        provider=ProviderConfig(model_context_windows={"local-model": 65_536}),
    )

    assert config.context.auto_compaction is True
    assert config.context.default_context_window_tokens == 32_768
    assert config.provider.model_context_windows["local-model"] == 65_536


def test_config_supports_memory_index_defaults_and_rejects_unknown_backends() -> None:
    config = ColossusConfig()

    assert config.memory.index.kind == "sqlite_fts"
    with pytest.raises(ValidationError):
        ColossusConfig.model_validate({"memory": {"index": {"kind": "chroma"}}})


def test_legacy_provider_config_maps_to_default_model_roles() -> None:
    config = ColossusConfig(
        provider=ProviderConfig(
            kind="local_openai_chat",
            model="primary-model",
            base_url="http://localhost:12434/v1",
        )
    )

    routing = effective_model_routing(config)

    assert routing.roles["primary"] == "primary"
    assert routing.roles["risk_evaluator"] == "primary"
    assert routing.roles["context_summarizer"] == "primary"
    assert routing.roles["subagent_default"] == "primary"
    assert routing.profiles["primary"].model == "primary-model"


def test_model_routing_supports_multiple_profiles_and_cli_primary_override() -> None:
    config = ColossusConfig(
        models=ModelRoutingConfig(
            profiles={
                "main": ModelProfile(provider="echo", model="main-model"),
                "risk": ModelProfile(
                    provider="local_openai_chat",
                    model="risk-model",
                    base_url="http://localhost:12434/v1",
                    context_window_tokens=16_384,
                ),
            },
            roles={
                "primary": "main",
                "risk_evaluator": "risk",
                "context_summarizer": "main",
                "subagent_default": "main",
            },
        )
    )

    routing = effective_model_routing(config, ProviderOverrides(model="override-model"))

    assert routing.profiles["main"].model == "override-model"
    assert routing.profiles["risk"].model == "risk-model"
    assert routing.roles["risk_evaluator"] == "risk"


def test_model_routing_primary_override_can_set_context_window() -> None:
    config = ColossusConfig(
        models=ModelRoutingConfig(
            profiles={"main": ModelProfile(provider="echo", model="main-model")},
            roles={"primary": "main"},
        )
    )

    routing = effective_model_routing(
        config,
        ProviderOverrides(model="override-model", context_window_tokens=131_072),
    )

    assert routing.profiles["main"].model == "override-model"
    assert routing.profiles["main"].context_window_tokens == 131_072


def test_model_routing_rejects_unknown_profile_references() -> None:
    config = ColossusConfig(
        models=ModelRoutingConfig(
            profiles={"main": ModelProfile(provider="echo", model="main-model")},
            roles={"primary": "main", "risk_evaluator": "missing"},
        ),
    )

    try:
        effective_model_routing(config)
    except ValueError as exc:
        assert "unknown profile" in str(exc)
    else:  # pragma: no cover
        raise AssertionError("Expected unknown profile to be rejected.")
