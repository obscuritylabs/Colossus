"""Search index port for durable memories."""

from typing import Protocol

from colossus.domain.memories import MemoryItem


class MemoryIndex(Protocol):
    async def upsert_memory_index(self, memory: MemoryItem) -> None:
        """Add or replace a memory in the retrieval index."""
        ...

    async def delete_memory_index(self, memory_id: str) -> None:
        """Remove a memory from the retrieval index."""
        ...

    async def search_memory_index(self, query: str, *, limit: int = 20) -> tuple[str, ...]:
        """Return candidate memory ids ordered by relevance."""
        ...
