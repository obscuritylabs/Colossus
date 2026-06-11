"""Filesystem skill repository."""

from pathlib import Path

from colossus.domain.skills import Skill, SkillManifest


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
            manifest_path = child / "manifest.json"
            skill_path = child / "SKILL.md"
            if child.is_dir() and manifest_path.is_file() and skill_path.is_file():
                manifest_text = manifest_path.read_text(encoding="utf-8")
                manifest = SkillManifest.model_validate_json(manifest_text)
                skills.append(
                    Skill(
                        manifest=manifest,
                        instructions=skill_path.read_text(encoding="utf-8"),
                        source=str(child),
                    )
                )
        return tuple(sorted(skills, key=lambda skill: skill.manifest.name))

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None
