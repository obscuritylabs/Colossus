"""Ports for research source collection."""

from typing import Protocol

from colossus.domain.research import ResearchSourceDraft


class RepoResearchProvider(Protocol):
    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        """Collect bounded local repository evidence for a query."""
        ...


class SearchProvider(Protocol):
    @property
    def configured(self) -> bool:
        """Whether web search is configured and usable."""
        ...

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        """Collect bounded web search evidence for a query."""
        ...


class McpGateway(Protocol):
    @property
    def configured(self) -> bool:
        """Whether MCP calls are configured and usable."""
        ...

    async def list_servers(self) -> tuple[dict[str, object], ...]:
        """List configured MCP servers."""
        ...

    async def list_tools(self, server: str | None = None) -> tuple[dict[str, object], ...]:
        """List tools exposed by configured MCP servers."""
        ...

    async def call_tool(
        self,
        *,
        server: str,
        tool: str,
        arguments: dict[str, object],
    ) -> dict[str, object]:
        """Call an approved MCP tool through a configured gateway."""
        ...

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        """Collect bounded MCP-backed evidence for a research query."""
        ...
