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


class WorkspaceSkillRepository:
    def __init__(self, workspace_root: Path) -> None:
        self._workspace_root = workspace_root.resolve(strict=False)

    def list_skills(self) -> tuple[Skill, ...]:
        skills: list[Skill] = []
        for root in workspace_skill_roots(self._workspace_root):
            skills.extend(FilesystemSkillRepository(root).list_skills())
        return tuple(skills)

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None


def workspace_skill_roots(workspace_root: Path) -> tuple[Path, ...]:
    workspace = workspace_root.resolve(strict=False)
    repo_root = _git_root_for(workspace)
    if repo_root is None:
        return (workspace / ".agents" / "skills",)
    roots: list[Path] = []
    current = repo_root
    roots.append(current / ".agents" / "skills")
    try:
        relative = workspace.relative_to(repo_root)
    except ValueError:
        return tuple(roots)
    for part in relative.parts:
        current = current / part
        roots.append(current / ".agents" / "skills")
    return tuple(roots)


def _git_root_for(path: Path) -> Path | None:
    current = path
    if current.is_file():
        current = current.parent
    for candidate in (current, *current.parents):
        if (candidate / ".git").exists():
            return candidate
    return None
