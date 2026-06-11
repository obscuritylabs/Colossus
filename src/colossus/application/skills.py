"""Skill resolution helpers."""

from colossus.domain.skills import Skill
from colossus.ports.skills import SkillRepository


class SkillResolver:
    def __init__(
        self,
        repositories: tuple[SkillRepository, ...],
        allow_user_overrides: bool = False,
    ) -> None:
        self._repositories = repositories
        self._allow_user_overrides = allow_user_overrides

    def list_skills(self) -> tuple[Skill, ...]:
        merged: dict[str, Skill] = {}
        for repository in self._repositories:
            for skill in repository.list_skills():
                if skill.manifest.name in merged and not self._allow_user_overrides:
                    continue
                merged[skill.manifest.name] = skill
        return tuple(merged.values())

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None
