"""Bundled package skill repository."""

import json
from importlib import resources

from colossus.domain.skills import Skill, SkillManifest


class PackageSkillRepository:
    def __init__(self, package: str = "colossus.bundled_skills") -> None:
        self._package = package

    def list_skills(self) -> tuple[Skill, ...]:
        root = resources.files(self._package)
        skills: list[Skill] = []
        for child in root.iterdir():
            if child.is_dir() and (child / "manifest.json").is_file():
                manifest_data = json.loads((child / "manifest.json").read_text(encoding="utf-8"))
                instructions = (child / "SKILL.md").read_text(encoding="utf-8")
                skills.append(
                    Skill(
                        manifest=SkillManifest.model_validate(manifest_data),
                        instructions=instructions,
                        source=f"package:{child.name}",
                    )
                )
        return tuple(sorted(skills, key=lambda skill: skill.manifest.name))

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None
