"""Skill authoring helpers shared by CLI, REPL, and tools."""

import json
import re
from dataclasses import dataclass
from pathlib import Path

from colossus.domain.errors import ColossusError
from colossus.domain.skills import SkillManifest

_SKILL_NAME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9_.-]*$")


@dataclass(frozen=True)
class SkillScaffoldResult:
    name: str
    path: Path
    manifest: SkillManifest


@dataclass(frozen=True)
class SkillValidationResult:
    path: Path
    valid: bool
    manifest: SkillManifest | None
    errors: tuple[str, ...]


class SkillAuthoringService:
    def __init__(self, user_skill_root: Path) -> None:
        self._user_skill_root = user_skill_root

    @property
    def user_skill_root(self) -> Path:
        return self._user_skill_root

    def scaffold(
        self,
        name: str,
        *,
        description: str | None = None,
        parent: Path | None = None,
        overwrite: bool = False,
    ) -> SkillScaffoldResult:
        normalized = _normalize_skill_name(name)
        target_root = parent or self._user_skill_root
        path = (target_root / normalized).resolve(strict=False)
        if path.exists() and not overwrite:
            raise ColossusError(
                f"Skill directory already exists: {path}. Use --force to overwrite."
            )
        manifest = SkillManifest(
            name=normalized,
            version="0.1.0",
            description=description or f"{normalized} skill.",
            triggers=_triggers_from_name(normalized),
            required_tools=(),
            permissions=(),
            offline_compatible=True,
        )
        path.mkdir(parents=True, exist_ok=True)
        (path / "manifest.json").write_text(
            f"{json.dumps(manifest.model_dump(mode='json'), indent=2)}\n",
            encoding="utf-8",
        )
        (path / "SKILL.md").write_text(_skill_template(normalized), encoding="utf-8")
        return SkillScaffoldResult(name=normalized, path=path, manifest=manifest)

    def scaffold_user_skill(
        self,
        name: str,
        *,
        description: str | None = None,
        overwrite: bool = False,
    ) -> SkillScaffoldResult:
        return self.scaffold(
            name,
            description=description,
            parent=self._user_skill_root,
            overwrite=overwrite,
        )

    def validate(self, path: Path) -> SkillValidationResult:
        resolved = path.resolve(strict=False)
        errors: list[str] = []
        manifest: SkillManifest | None = None
        if not resolved.exists():
            return SkillValidationResult(
                path=resolved,
                valid=False,
                manifest=None,
                errors=("Skill directory does not exist.",),
            )
        if not resolved.is_dir():
            return SkillValidationResult(
                path=resolved,
                valid=False,
                manifest=None,
                errors=("Skill path is not a directory.",),
            )
        manifest_path = resolved / "manifest.json"
        skill_path = resolved / "SKILL.md"
        if not manifest_path.is_file():
            errors.append("manifest.json is missing.")
        else:
            try:
                manifest = SkillManifest.model_validate_json(
                    manifest_path.read_text(encoding="utf-8")
                )
            except ValueError as exc:
                errors.append(f"manifest.json is invalid: {exc}")
        if manifest is not None and not _SKILL_NAME_RE.fullmatch(manifest.name):
            errors.append(
                "manifest.json name must start with a letter and contain only "
                "letters, numbers, dots, underscores, or hyphens."
            )
        if not skill_path.is_file():
            errors.append("SKILL.md is missing.")
        else:
            skill_text = skill_path.read_text(encoding="utf-8").strip()
            if not skill_text:
                errors.append("SKILL.md is empty.")
        return SkillValidationResult(
            path=resolved,
            valid=not errors,
            manifest=manifest,
            errors=tuple(errors),
        )

    def validate_user_skill(self, name: str) -> SkillValidationResult:
        normalized = _normalize_skill_name(name)
        return self.validate(self._user_skill_root / normalized)


def _normalize_skill_name(name: str) -> str:
    normalized = name.strip()
    if not _SKILL_NAME_RE.fullmatch(normalized):
        raise ColossusError(
            "Skill name must start with a letter and contain only letters, numbers, "
            "dots, underscores, or hyphens."
        )
    return normalized


def _triggers_from_name(name: str) -> tuple[str, ...]:
    parts = tuple(part for part in re.split(r"[._-]+", name) if part)
    return _dedupe((name, *parts))


def _dedupe(values: tuple[str, ...]) -> tuple[str, ...]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value not in seen:
            seen.add(value)
            result.append(value)
    return tuple(result)


def _skill_template(name: str) -> str:
    title = name.replace("-", " ").replace("_", " ").replace(".", " ").title()
    return (
        f"# {title}\n\n"
        "Use this skill when the user asks for this workflow. Start by identifying the\n"
        "goal, constraints, source files, and expected output. Use the smallest relevant\n"
        "tool set, keep changes focused, and validate the result before reporting back.\n"
    )
