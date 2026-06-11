"""Deterministic offline provider used for smoke tests and empty config."""

from collections.abc import AsyncIterator

from colossus.domain.events import FinalOutputEvent, ModelDeltaEvent, RunEvent
from colossus.domain.messages import UserMessage
from colossus.domain.providers import (
    ProviderCapability,
    ProviderModelInfo,
    ProviderReadiness,
    ProviderReadinessCheck,
)
from colossus.domain.requests import ModelRequest


class EchoModelProvider:
    name = "echo"

    def capabilities(self) -> tuple[ProviderCapability, ...]:
        return (
            ProviderCapability(
                name="offline",
                supported=True,
                detail="Deterministic provider with no network dependency.",
            ),
            ProviderCapability(
                name="tool_calls",
                supported=False,
                detail="Echo replies do not request tools.",
            ),
        )

    async def check_readiness(self) -> ProviderReadiness:
        return ProviderReadiness(
            provider=self.name,
            ready=True,
            checks=(
                ProviderReadinessCheck(
                    name="offline",
                    status="pass",
                    detail="Echo provider is always available.",
                ),
            ),
        )

    async def list_models(self) -> tuple[ProviderModelInfo, ...]:
        return (ProviderModelInfo(id="default", owner="colossus"),)

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        last_user = next(
            (
                message.content
                for message in reversed(request.messages)
                if isinstance(message, UserMessage)
            ),
            "",
        )
        text = f"[echo:{request.model}] {last_user}"
        yield ModelDeltaEvent(text=text)
        yield FinalOutputEvent(text=text)
