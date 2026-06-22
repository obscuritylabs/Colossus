"""Application service for durable memories."""

from uuid import uuid4

from colossus.domain.errors import ColossusError
from colossus.domain.memories import (
    MemoryItem,
    MemoryKind,
    MemoryScope,
    MemorySource,
    MemoryStatus,
    utc_now_iso,
)
from colossus.ports.audit import AuditSink
from colossus.ports.memory_index import MemoryIndex
from colossus.ports.state import StateStore


class MemoryService:
    def __init__(
        self,
        state_store: StateStore,
        audit_sink: AuditSink,
        memory_index: MemoryIndex | None = None,
    ) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink
        self._memory_index = memory_index

    async def create_memory(
        self,
        *,
        scope: MemoryScope,
        kind: MemoryKind,
        text: str,
        source: MemorySource = "agent",
        confidence: float = 1.0,
        rationale: str = "",
        repo_root: str | None = None,
        session_id: str | None = None,
        supersedes: str | None = None,
        stale_after: str | None = None,
        expires_at: str | None = None,
        memory_id: str | None = None,
    ) -> MemoryItem:
        self._validate_scope(scope, repo_root=repo_root, session_id=session_id)
        if not text:
            raise ColossusError("Memory text is required.")
        resolved_id = memory_id or f"mem_{uuid4().hex[:12]}"
        existing = await self._state_store.get_memory(resolved_id)
        if existing is not None:
            raise ColossusError(f"Memory already exists: {resolved_id}")
        if session_id is not None:
            await self._state_store.ensure_session(session_id, title=text[:80])
        now = utc_now_iso()
        memory = MemoryItem(
            id=resolved_id,
            scope=scope,
            kind=kind,
            status="active",
            source=source,
            confidence=confidence,
            text=text,
            rationale=rationale,
            repo_root=repo_root,
            session_id=session_id,
            supersedes=supersedes,
            stale_after=stale_after,
            expires_at=expires_at,
            created_at=now,
            updated_at=now,
        )
        await self._save_and_index(memory)
        await self._audit_sink.record(
            source,
            "memory.created",
            {
                "memory_id": memory.id,
                "scope": memory.scope,
                "kind": memory.kind,
                "source": memory.source,
                "repo_root": memory.repo_root,
                "session_id": memory.session_id,
            },
        )
        return memory

    async def update_memory(
        self,
        memory_id: str,
        *,
        scope: MemoryScope | None = None,
        kind: MemoryKind | None = None,
        text: str | None = None,
        status: MemoryStatus | None = None,
        confidence: float | None = None,
        rationale: str | None = None,
        repo_root: str | None = None,
        session_id: str | None = None,
        stale_after: str | None = None,
        expires_at: str | None = None,
    ) -> MemoryItem:
        memory = await self._require_memory(memory_id)
        resolved_scope = scope or memory.scope
        resolved_repo = repo_root if repo_root is not None else memory.repo_root
        resolved_session = session_id if session_id is not None else memory.session_id
        self._validate_scope(resolved_scope, repo_root=resolved_repo, session_id=resolved_session)
        changes: dict[str, object | None] = {"updated_at": utc_now_iso()}
        if scope is not None:
            changes["scope"] = scope
        if kind is not None:
            changes["kind"] = kind
        if text is not None:
            if not text:
                raise ColossusError("Memory text cannot be empty.")
            changes["text"] = text
        if status is not None:
            changes["status"] = status
        if confidence is not None:
            changes["confidence"] = confidence
        if rationale is not None:
            changes["rationale"] = rationale
        if repo_root is not None:
            changes["repo_root"] = repo_root
        if session_id is not None:
            changes["session_id"] = session_id
        if stale_after is not None:
            changes["stale_after"] = stale_after
        if expires_at is not None:
            changes["expires_at"] = expires_at
        updated = memory.model_copy(update=changes)
        await self._save_and_index(updated)
        await self._audit_sink.record(
            "agent",
            "memory.updated",
            {
                "memory_id": updated.id,
                "scope": updated.scope,
                "kind": updated.kind,
                "status": updated.status,
            },
        )
        return updated

    async def archive_memory(self, memory_id: str) -> MemoryItem:
        archived = await self.update_memory(memory_id, status="archived")
        await self._audit_sink.record("agent", "memory.archived", {"memory_id": archived.id})
        return archived

    async def supersede_memory(
        self,
        memory_id: str,
        *,
        text: str,
        source: MemorySource = "agent",
        scope: MemoryScope | None = None,
        kind: MemoryKind | None = None,
        confidence: float | None = None,
        rationale: str = "",
        repo_root: str | None = None,
        session_id: str | None = None,
        stale_after: str | None = None,
        expires_at: str | None = None,
    ) -> MemoryItem:
        old = await self._require_memory(memory_id)
        await self.update_memory(old.id, status="superseded")
        replacement = await self.create_memory(
            scope=scope or old.scope,
            kind=kind or old.kind,
            text=text,
            source=source,
            confidence=confidence if confidence is not None else old.confidence,
            rationale=rationale,
            repo_root=repo_root if repo_root is not None else old.repo_root,
            session_id=session_id if session_id is not None else old.session_id,
            supersedes=old.id,
            stale_after=stale_after if stale_after is not None else old.stale_after,
            expires_at=expires_at if expires_at is not None else old.expires_at,
        )
        await self._audit_sink.record(
            source,
            "memory.superseded",
            {"memory_id": old.id, "replacement_id": replacement.id},
        )
        return replacement

    async def get_memory(self, memory_id: str) -> MemoryItem:
        return await self._require_memory(memory_id)

    async def list_memories(
        self,
        *,
        scope: MemoryScope | None = None,
        kind: MemoryKind | None = None,
        status: MemoryStatus | None = "active",
        repo_root: str | None = None,
        session_id: str | None = None,
    ) -> tuple[MemoryItem, ...]:
        return await self._state_store.list_memories(
            scope=scope,
            kind=kind,
            status=status,
            repo_root=repo_root,
            session_id=session_id,
        )

    async def search_memories(
        self,
        query: str,
        *,
        repo_root: str | None = None,
        session_id: str | None = None,
        kind: MemoryKind | None = None,
        status: MemoryStatus | None = "active",
        limit: int = 8,
    ) -> tuple[MemoryItem, ...]:
        if self._memory_index is None or not query.strip():
            return await self._fallback_relevant_memories(
                repo_root=repo_root,
                session_id=session_id,
                kind=kind,
                status=status,
                limit=limit,
            )
        candidate_ids = await self._memory_index.search_memory_index(
            query,
            limit=max(limit * 4, limit),
        )
        matches: list[MemoryItem] = []
        for candidate_id in candidate_ids:
            memory = await self._state_store.get_memory(candidate_id)
            if memory is None:
                continue
            if not self._memory_matches(
                memory,
                repo_root=repo_root,
                session_id=session_id,
                kind=kind,
                status=status,
            ):
                continue
            matches.append(memory)
            if len(matches) >= limit:
                break
        if matches:
            return tuple(sorted(matches, key=lambda memory: self._scope_rank(memory)))
        return ()

    async def relevant_memories(
        self,
        query: str,
        *,
        repo_root: str | None = None,
        session_id: str | None = None,
        limit: int = 6,
    ) -> tuple[MemoryItem, ...]:
        return await self.search_memories(
            query,
            repo_root=repo_root,
            session_id=session_id,
            status="active",
            limit=limit,
        )

    async def _save_and_index(self, memory: MemoryItem) -> None:
        await self._state_store.save_memory(memory)
        if self._memory_index is None:
            return
        if memory.status == "active":
            await self._memory_index.upsert_memory_index(memory)
            return
        await self._memory_index.delete_memory_index(memory.id)

    async def _fallback_relevant_memories(
        self,
        *,
        repo_root: str | None,
        session_id: str | None,
        kind: MemoryKind | None,
        status: MemoryStatus | None,
        limit: int,
    ) -> tuple[MemoryItem, ...]:
        candidates = await self._state_store.list_memories(status=status, kind=kind)
        filtered = [
            memory
            for memory in candidates
            if self._memory_matches(
                memory,
                repo_root=repo_root,
                session_id=session_id,
                kind=kind,
                status=status,
            )
        ]
        return tuple(sorted(filtered, key=lambda memory: self._scope_rank(memory))[:limit])

    async def _require_memory(self, memory_id: str) -> MemoryItem:
        memory = await self._state_store.get_memory(memory_id)
        if memory is None:
            raise ColossusError(f"Memory not found: {memory_id}")
        return memory

    @staticmethod
    def _validate_scope(
        scope: MemoryScope,
        *,
        repo_root: str | None,
        session_id: str | None,
    ) -> None:
        if scope == "repo" and not repo_root:
            raise ColossusError("Repo-scoped memories require repo_root.")
        if scope == "session" and not session_id:
            raise ColossusError("Session-scoped memories require session_id.")

    @staticmethod
    def _memory_matches(
        memory: MemoryItem,
        *,
        repo_root: str | None,
        session_id: str | None,
        kind: MemoryKind | None,
        status: MemoryStatus | None,
    ) -> bool:
        if status is not None and memory.status != status:
            return False
        if kind is not None and memory.kind != kind:
            return False
        if memory.scope == "global":
            return True
        if memory.scope == "repo":
            return repo_root is not None and memory.repo_root == repo_root
        if memory.scope == "session":
            return session_id is not None and memory.session_id == session_id
        return False

    @staticmethod
    def _scope_rank(memory: MemoryItem) -> tuple[int, str, str]:
        ranks = {"repo": 0, "session": 1, "global": 2}
        return (ranks[memory.scope], memory.created_at, memory.id)
