"""Application service for session discovery and resume selection."""

from collections.abc import Callable
from uuid import uuid4

from colossus.domain.errors import ColossusError
from colossus.domain.messages import Message
from colossus.domain.sessions import SessionSummary
from colossus.ports.state import StateStore

SessionIdFactory = Callable[[], str]


class SessionService:
    def __init__(
        self,
        state_store: StateStore,
        *,
        session_id_factory: SessionIdFactory | None = None,
    ) -> None:
        self._state_store = state_store
        self._session_id_factory = session_id_factory or (lambda: str(uuid4()))

    def new_session_id(self) -> str:
        return self._session_id_factory()

    async def get_session(self, session_id: str) -> SessionSummary | None:
        return await self._state_store.get_session(session_id)

    async def require_session(self, session_id: str) -> SessionSummary:
        session = await self.get_session(session_id)
        if session is None:
            raise ColossusError(f"Session not found: {session_id}")
        return session

    async def list_sessions(self, limit: int = 20) -> tuple[SessionSummary, ...]:
        return await self._state_store.list_sessions(limit)

    async def latest_session(self) -> SessionSummary:
        sessions = await self.list_sessions(limit=1)
        if not sessions:
            raise ColossusError("No sessions exist yet. Start a run before using --resume.")
        return sessions[0]

    async def recent_messages(self, session_id: str, limit: int = 10) -> tuple[Message, ...]:
        safe_limit = max(limit, 0)
        messages = await self._state_store.list_messages(session_id)
        if safe_limit == 0:
            return ()
        return messages[-safe_limit:]
