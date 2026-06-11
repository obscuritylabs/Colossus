"""Audit sink port."""

from typing import Protocol

from colossus.domain.audit import AuditRecord


class AuditSink(Protocol):
    async def record(self, actor: str, event: str, details: dict[str, object]) -> AuditRecord:
        """Append an audit record and return the stored record."""
        ...
