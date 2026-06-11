"""Port for structured user prompts in interactive surfaces."""

from typing import Protocol

from colossus.domain.user_prompts import UserPromptAnswer, UserPromptChoice


class UserPromptHandler(Protocol):
    async def ask(
        self,
        *,
        question: str,
        choices: tuple[UserPromptChoice, ...] = (),
        allow_freeform: bool = True,
    ) -> UserPromptAnswer:
        """Ask the user a structured question and return their answer."""
        ...
