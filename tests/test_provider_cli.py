from collections.abc import AsyncIterator

import httpx
import pytest
from typer.testing import CliRunner

import colossus.cli as cli_module
from colossus.adapters.openai_compat import LocalOpenAIChatProvider, OpenAIResponsesProvider
from colossus.application.providers import ProviderDiagnostics
from colossus.cli import app
from colossus.domain.events import FinalOutputEvent, RunEvent, ToolCallRequestedEvent
from colossus.domain.providers import (
    ProviderCapability,
    ProviderModelInfo,
    ProviderReadiness,
    ProviderReadinessCheck,
)
from colossus.domain.requests import ModelRequest


class ToolCallingProbeProvider:
    name = "tool-calling-probe"

    def capabilities(self) -> tuple[ProviderCapability, ...]:
        return (
            ProviderCapability(
                name="tool_calls",
                supported=True,
                detail="Test provider emits tool calls.",
            ),
        )

    async def check_readiness(self) -> ProviderReadiness:
        return ProviderReadiness(
            provider=self.name,
            ready=True,
            checks=(
                ProviderReadinessCheck(
                    name="test",
                    status="pass",
                    detail="ready",
                ),
            ),
        )

    async def list_models(self) -> tuple[ProviderModelInfo, ...]:
        return (ProviderModelInfo(id="probe-model"),)

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        yield ToolCallRequestedEvent(
            call_id="probe-call",
            name="colossus_tool_probe",
            arguments={"token": "probe-ok"},
        )


class ToolCallingProbeThenFinalProvider(ToolCallingProbeProvider):
    def __init__(self) -> None:
        self.consumed_after_probe = False

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        yield ToolCallRequestedEvent(
            call_id="probe-call",
            name="colossus_tool_probe",
            arguments={"token": "probe-ok"},
        )
        self.consumed_after_probe = True
        yield FinalOutputEvent(text="done")


def test_provider_doctor_reports_echo_ready(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    result = CliRunner().invoke(app, ["provider", "doctor"])

    assert result.exit_code == 0
    assert "Provider: echo" in result.stdout
    assert "Status: ready" in result.stdout
    assert "offline" in result.stdout


def test_provider_models_lists_echo_model(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    result = CliRunner().invoke(app, ["provider", "models"])

    assert result.exit_code == 0
    assert "default" in result.stdout
    assert "colossus" in result.stdout


def test_provider_models_lists_discovered_limits(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    class LimitProvider(ToolCallingProbeProvider):
        async def list_models(self) -> tuple[ProviderModelInfo, ...]:
            return (
                ProviderModelInfo(
                    id="limit-model",
                    context_window_tokens=131_072,
                    max_output_tokens=8_192,
                ),
            )

    monkeypatch.setattr(cli_module, "provider_from_config", lambda *args, **kwargs: LimitProvider())

    result = CliRunner().invoke(app, ["provider", "models"])

    assert result.exit_code == 0
    assert "limit-model" in result.stdout
    assert "131072" in result.stdout
    assert "8192" in result.stdout


def test_provider_doctor_reports_missing_openai_api_key(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    result = CliRunner().invoke(app, ["--provider", "openai-responses", "provider", "doctor"])

    assert result.exit_code == 1
    assert "Provider: openai-responses" in result.stdout
    assert "Status: not ready" in result.stdout
    assert "API key is not configured" in result.stdout


def test_provider_doctor_can_probe_model_tool_calls(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    result = CliRunner().invoke(app, ["provider", "doctor", "--probe-tools"])

    assert result.exit_code == 0
    assert "model_tool_calls" in result.stdout
    assert "execute tools from text" in result.stdout


@pytest.mark.asyncio
async def test_provider_diagnostics_tool_call_probe_passes() -> None:
    check = await ProviderDiagnostics(ToolCallingProbeProvider()).probe_tool_calls("probe-model")

    assert check.name == "model_tool_calls"
    assert check.status == "pass"
    assert "probe-model" in check.detail


@pytest.mark.asyncio
async def test_provider_diagnostics_tool_call_probe_drains_stream() -> None:
    provider = ToolCallingProbeThenFinalProvider()

    check = await ProviderDiagnostics(provider).probe_tool_calls("probe-model")

    assert check.status == "pass"
    assert provider.consumed_after_probe is True


@pytest.mark.asyncio
async def test_openai_responses_provider_lists_models() -> None:
    transport = httpx.MockTransport(
        lambda request: httpx.Response(
            200,
            json={
                "data": [
                    {"id": "gpt-test", "owned_by": "openai", "created": 123},
                    {"id": "missing-metadata"},
                ]
            },
            request=request,
        )
    )
    provider = OpenAIResponsesProvider(
        api_key="test-key",
        base_url="https://api.example.test/v1",
        transport=transport,
    )

    models = await provider.list_models()

    assert [model.id for model in models] == ["gpt-test", "missing-metadata"]
    assert models[0].owner == "openai"
    assert models[0].created == 123


@pytest.mark.asyncio
async def test_openai_responses_provider_extracts_model_limits() -> None:
    transport = httpx.MockTransport(
        lambda request: httpx.Response(
            200,
            json={
                "data": [
                    {
                        "id": "openrouter-model",
                        "context_length": 131_072,
                        "top_provider": {"max_completion_tokens": 8_192},
                    }
                ]
            },
            request=request,
        )
    )
    provider = OpenAIResponsesProvider(
        api_key="test-key",
        base_url="https://api.example.test/v1",
        transport=transport,
    )

    models = await provider.list_models()

    assert models[0].context_window_tokens == 131_072
    assert models[0].max_output_tokens == 8_192


@pytest.mark.asyncio
async def test_local_openai_chat_provider_extracts_openrouter_model_limits() -> None:
    transport = httpx.MockTransport(
        lambda request: httpx.Response(
            200,
            json={
                "data": [
                    {
                        "id": "openrouter/chat-model",
                        "top_provider": {
                            "context_length": 200_000,
                            "max_completion_tokens": 16_384,
                        },
                    }
                ]
            },
            request=request,
        )
    )
    provider = LocalOpenAIChatProvider(
        base_url="https://openrouter.ai/api/v1",
        transport=transport,
    )

    models = await provider.list_models()

    assert models[0].context_window_tokens == 200_000
    assert models[0].max_output_tokens == 16_384


@pytest.mark.asyncio
async def test_local_openai_chat_provider_doctor_uses_models_endpoint() -> None:
    transport = httpx.MockTransport(
        lambda request: httpx.Response(200, json={"data": []}, request=request)
    )
    provider = LocalOpenAIChatProvider(
        base_url="http://localhost:8000/v1",
        transport=transport,
    )

    readiness = await provider.check_readiness()

    assert readiness.ready is True
    assert readiness.checks[0].name == "models_endpoint"
