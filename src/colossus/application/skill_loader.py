"""Shared skill loading helpers."""

import re
from importlib.resources.abc import Traversable
from pathlib import Path

from colossus.domain.errors import ColossusError
from colossus.domain.skills import Skill, SkillManifest

_SKILL_NAME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9_.-]*$")
_FRONTMATTER_RE = re.compile(r"\A---\n(?P<meta>.*?)\n---\n?", re.DOTALL)


def load_skill_from_directory(root: Path | Traversable, *, source: str) -> Skill | None:
    skill_path = root / "SKILL.md"
    manifest_path = root / "manifest.json"
    if not skill_path.is_file():
        return None
    skill_text = skill_path.read_text(encoding="utf-8")
    frontmatter, instructions = parse_skill_frontmatter(skill_text)
    if manifest_path.is_file():
        manifest = SkillManifest.model_validate_json(manifest_path.read_text(encoding="utf-8"))
        _validate_frontmatter_matches_manifest(frontmatter, manifest)
    else:
        manifest = manifest_from_frontmatter(frontmatter)
    return Skill(
        manifest=manifest,
        instructions=instructions,
        source=source,
        resource_root=_resource_root_string(root),
    )


def parse_skill_frontmatter(text: str) -> tuple[dict[str, str], str]:
    match = _FRONTMATTER_RE.match(text)
    if match is None:
        return {}, text
    metadata = _parse_simple_yaml_mapping(match.group("meta"))
    return metadata, text[match.end() :].lstrip("\n")


def manifest_from_frontmatter(frontmatter: dict[str, str]) -> SkillManifest:
    name = frontmatter.get("name", "").strip()
    description = frontmatter.get("description", "").strip()
    if not name:
        raise ColossusError("SKILL.md frontmatter must include name.")
    if not description:
        raise ColossusError("SKILL.md frontmatter must include description.")
    if not _SKILL_NAME_RE.fullmatch(name):
        raise ColossusError(
            "SKILL.md frontmatter name must start with a letter and contain only "
            "letters, numbers, dots, underscores, or hyphens."
        )
    return SkillManifest(
        name=name,
        version="0.1.0",
        description=description,
        triggers=_triggers_from_name(name),
        required_tools=(),
        permissions=(),
        offline_compatible=True,
    )


def _validate_frontmatter_matches_manifest(
    frontmatter: dict[str, str],
    manifest: SkillManifest,
) -> None:
    frontmatter_name = frontmatter.get("name")
    if frontmatter_name is not None and frontmatter_name != manifest.name:
        raise ColossusError(
            f"SKILL.md frontmatter name {frontmatter_name!r} does not match "
            f"manifest.json name {manifest.name!r}."
        )
    frontmatter_description = frontmatter.get("description")
    if (
        frontmatter_description is not None
        and frontmatter_description != manifest.description
    ):
        raise ColossusError(
            "SKILL.md frontmatter description does not match manifest.json description."
        )


def _parse_simple_yaml_mapping(text: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition(":")
        if not separator:
            continue
        normalized_key = key.strip()
        if normalized_key in {"name", "description"}:
            values[normalized_key] = _yaml_scalar(value.strip())
    return values


def _yaml_scalar(value: str) -> str:
    if (
        len(value) >= 2
        and value[0] == value[-1]
        and value[0] in {"'", '"'}
    ):
        return value[1:-1]
    return value


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


def _resource_root_string(root: Path | Traversable) -> str | None:
    if isinstance(root, Path):
        return str(root.resolve(strict=False))
    return None
