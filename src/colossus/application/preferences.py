"""Application service for user preferences."""

from colossus.domain.preferences import ReplPreferences
from colossus.ports.state import StateStore

DEFAULT_REPL_PREFERENCES_PROFILE = "default"


class ReplPreferencesService:
    def __init__(
        self,
        state_store: StateStore,
        *,
        profile: str = DEFAULT_REPL_PREFERENCES_PROFILE,
    ) -> None:
        self._state_store = state_store
        self._profile = profile

    async def load(self) -> ReplPreferences:
        stored = await self._state_store.get_repl_preferences(self._profile)
        return stored or ReplPreferences()

    async def save(self, preferences: ReplPreferences) -> ReplPreferences:
        await self._state_store.save_repl_preferences(self._profile, preferences)
        return preferences

    async def reset(self) -> ReplPreferences:
        preferences = ReplPreferences()
        await self.save(preferences)
        return preferences
