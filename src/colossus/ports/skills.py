"""Skill repository port."""

from typing import Protocol

from colossus.domain.skills import Skill


class SkillRepository(Protocol):
    def list_skills(self) -> tuple[Skill, ...]:
        """Return enabled skills in precedence order."""
        ...

    def get_skill(self, name: str) -> Skill | None:
        """Return a skill by name."""
        ...
