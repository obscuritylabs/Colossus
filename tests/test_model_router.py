from colossus.adapters.echo_provider import EchoModelProvider
from colossus.adapters.openai_compat import LocalOpenAIChatProvider
from colossus.domain.models import ModelProfile, ModelRoutingConfig
from colossus.infrastructure.config import ColossusConfig
from colossus.infrastructure.container import create_model_router


def test_model_router_resolves_configured_roles() -> None:
    config = ColossusConfig(
        models=ModelRoutingConfig(
            profiles={
                "main": ModelProfile(provider="echo", model="main-model"),
                "risk": ModelProfile(
                    provider="local_openai_chat",
                    model="risk-model",
                    base_url="http://localhost:12434/v1",
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

    router = create_model_router(config, require_credentials=False)
    primary = router.resolve("primary")
    risk = router.resolve("risk_evaluator")
    research = router.resolve("research_synthesizer")

    assert primary.profile.model == "main-model"
    assert isinstance(primary.provider, EchoModelProvider)
    assert risk.profile.model == "risk-model"
    assert isinstance(risk.provider, LocalOpenAIChatProvider)
    assert risk.provider.base_url == "http://localhost:12434/v1"
    assert research.profile.model == "main-model"
