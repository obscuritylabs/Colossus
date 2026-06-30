"""Filesystem skill repository."""

from pathlib import Path

from colossus.application.skill_loader import load_skill_from_directory
from colossus.domain.skills import Skill


class FilesystemSkillRepository:
    def __init__(self, root: Path, disabled: frozenset[str] | None = None) -> None:
        self._root = root
        self._disabled = disabled or frozenset()

    def list_skills(self) -> tuple[Skill, ...]:
        if not self._root.exists():
            return ()
        skills: list[Skill] = []
        for child in self._root.iterdir():
            if child.name in self._disabled:
                continue
            if child.is_dir():
                skill = load_skill_from_directory(child, source=str(child))
                if skill is not None:
                    skills.append(skill)
        return tuple(sorted(skills, key=lambda skill: skill.manifest.name))

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None
