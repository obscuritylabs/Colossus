"""Bundled package skill repository."""

from importlib import resources

from colossus.application.skill_loader import load_skill_from_directory
from colossus.domain.skills import Skill


class PackageSkillRepository:
    def __init__(self, package: str = "colossus.bundled_skills") -> None:
        self._package = package

    def list_skills(self) -> tuple[Skill, ...]:
        root = resources.files(self._package)
        skills: list[Skill] = []
        for child in root.iterdir():
            if child.is_dir():
                skill = load_skill_from_directory(child, source=f"package:{child.name}")
                if skill is not None:
                    skills.append(skill)
        return tuple(sorted(skills, key=lambda skill: skill.manifest.name))

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None
