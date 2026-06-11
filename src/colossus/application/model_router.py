"""Provider-neutral routing for named model roles."""

from collections.abc import AsyncIterator, Mapping
from dataclasses import dataclass

from colossus.domain.errors import ColossusError
from colossus.domain.events import RunEvent
from colossus.domain.models import ResolvedModelProfile
from colossus.domain.requests import ModelRequest
from colossus.ports.model_provider import ModelProvider


@dataclass(frozen=True)
class ModelRoute:
    role: str
    profile_name: str
    provider: ModelProvider
    profile: ResolvedModelProfile


class ModelRouter:
    def __init__(self, routes: Mapping[str, ModelRoute]) -> None:
        self._routes = dict(routes)

    def resolve(self, role: str) -> ModelRoute:
        try:
            return self._routes[role]
        except KeyError as exc:
            raise ColossusError(f"Unknown model role: {role}") from exc

    def list_routes(self) -> tuple[ModelRoute, ...]:
        return tuple(self._routes[role] for role in sorted(self._routes))

    async def stream(self, role: str, request: ModelRequest) -> AsyncIterator[RunEvent]:
        route = self.resolve(role)
        routed_request = request.model_copy(update={"model": route.profile.model})
        async for event in route.provider.stream(routed_request):
            yield event
