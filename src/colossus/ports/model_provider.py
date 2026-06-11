"""Model provider port."""

from collections.abc import AsyncIterator
from typing import Protocol

from colossus.domain.events import RunEvent
from colossus.domain.providers import ProviderCapability, ProviderModelInfo, ProviderReadiness
from colossus.domain.requests import ModelRequest


class ModelProvider(Protocol):
    name: str

    def capabilities(self) -> tuple[ProviderCapability, ...]:
        """Describe static capabilities exposed by this provider."""
        ...

    async def check_readiness(self) -> ProviderReadiness:
        """Check whether this provider is currently usable."""
        ...

    async def list_models(self) -> tuple[ProviderModelInfo, ...]:
        """Return models advertised by this provider."""
        ...

    def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        """Yield normalized model events for one model turn."""
        ...
