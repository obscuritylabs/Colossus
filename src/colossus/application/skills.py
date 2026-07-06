"""Skill resolution and model-context helpers."""

import re
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

from pydantic import BaseModel, ConfigDict

from colossus.domain.agents import AgentSpec
from colossus.domain.errors import ColossusError
from colossus.domain.skills import Skill
from colossus.domain.tools import ToolSpec
from colossus.ports.skills import SkillRepository

_SKILL_MENTION_RE = re.compile(
    r"(?<![\w.])@(?:(?:skill:)(?P<canonical>[A-Za-z][A-Za-z0-9_.-]*)|"
    r"(?P<shorthand>[A-Za-z][A-Za-z0-9_.-]*))"
)
_RESOURCE_DIRS = frozenset({"references", "scripts", "assets", "examples", "tests"})
_MAX_RESOURCE_BYTES = 64_000
_TEXT_RESOURCE_SUFFIXES = frozenset(
    {
        ".json",
        ".md",
        ".py",
        ".sh",
        ".txt",
        ".toml",
        ".yaml",
        ".yml",
    }
)


class SkillResourceEntry(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    path: str
    size: int
    kind: str


class SkillResourceRead(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    path: str
    size: int
    content: str


@dataclass(frozen=True)
class SkillDuplicate:
    name: str
    selected_source: str
    sources: tuple[str, ...]


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

    def duplicate_names(self) -> tuple[SkillDuplicate, ...]:
        sources_by_name: dict[str, list[str]] = {}
        for repository in self._repositories:
            for skill in repository.list_skills():
                sources_by_name.setdefault(skill.manifest.name, []).append(skill.source)
        duplicates: list[SkillDuplicate] = []
        for name, sources in sorted(sources_by_name.items()):
            if len(sources) < 2:
                continue
            selected_source = sources[-1] if self._allow_user_overrides else sources[0]
            duplicates.append(
                SkillDuplicate(
                    name=name,
                    selected_source=selected_source,
                    sources=tuple(sources),
                )
            )
        return tuple(duplicates)


@dataclass(frozen=True)
class SkillComposition:
    instructions: str
    available_skills: tuple[Skill, ...]
    active_skills: tuple[Skill, ...]

    @property
    def active_metadata(self) -> tuple[dict[str, str], ...]:
        return tuple(
            {
                "name": skill.manifest.name,
                "version": skill.manifest.version,
                "source": skill.source,
            }
            for skill in self.active_skills
        )


class SkillComposer:
    def __init__(self, resolver: SkillResolver) -> None:
        self._resolver = resolver

    def compose(
        self,
        *,
        instructions: str,
        agent: AgentSpec,
        prompt: str,
        active_skills: tuple[str, ...],
        skill_mode_enabled: bool,
        tools: tuple[ToolSpec, ...],
    ) -> SkillComposition:
        all_skills = self._skills_by_name()
        prompt_skill_names = extract_skill_mentions(prompt, available_names=all_skills.keys())
        requested_names = _dedupe((*active_skills, *prompt_skill_names))
        if not skill_mode_enabled:
            if requested_names:
                raise ColossusError(
                    "Skill Mode is disabled; enable it before using skill mentions."
                )
            return SkillComposition(
                instructions=instructions,
                available_skills=(),
                active_skills=(),
            )

        available = self.available_skills(agent)
        available_by_name = {skill.manifest.name: skill for skill in available}
        unknown = tuple(name for name in requested_names if name not in all_skills)
        if unknown:
            raise ColossusError(f"Unknown skill: {', '.join(unknown)}")
        unavailable = tuple(name for name in requested_names if name not in available_by_name)
        if unavailable:
            raise ColossusError(f"Skill is not available to this agent: {', '.join(unavailable)}")

        selected = tuple(available_by_name[name] for name in requested_names)
        self._validate_required_tools(selected, tools)
        skill_context = _format_skill_context(available, selected)
        return SkillComposition(
            instructions=f"{instructions.rstrip()}\n\n{skill_context}",
            available_skills=available,
            active_skills=selected,
        )

    def available_skills(self, agent: AgentSpec) -> tuple[Skill, ...]:
        all_skills = self._skills_by_name()
        if not agent.skills:
            return tuple(all_skills.values())
        ordered_names = _dedupe(agent.skills)
        missing = tuple(name for name in ordered_names if name not in all_skills)
        if missing:
            raise ColossusError(f"Agent references unknown skills: {', '.join(missing)}")
        return tuple(all_skills[name] for name in ordered_names)

    def _skills_by_name(self) -> dict[str, Skill]:
        return {skill.manifest.name: skill for skill in self._resolver.list_skills()}

    def _validate_required_tools(
        self,
        skills: tuple[Skill, ...],
        tools: tuple[ToolSpec, ...],
    ) -> None:
        tool_names = {tool.name for tool in tools}
        for skill in skills:
            missing = tuple(
                tool for tool in skill.manifest.required_tools if tool not in tool_names
            )
            if missing:
                raise ColossusError(
                    f"Skill {skill.manifest.name} requires unavailable tools: "
                    f"{_format_missing_required_tools(missing, tool_names)}"
                )


class SkillResourceService:
    def __init__(self, resolver: SkillResolver) -> None:
        self._resolver = resolver

    def list_resources(
        self,
        *,
        skill_name: str,
        active_skills: tuple[str, ...],
    ) -> tuple[SkillResourceEntry, ...]:
        skill, root = self._active_skill_root(skill_name, active_skills)
        del skill
        resources: list[SkillResourceEntry] = []
        for resource_dir in sorted(_RESOURCE_DIRS):
            base = root / resource_dir
            if not base.is_dir():
                continue
            for path in sorted(item for item in base.rglob("*") if item.is_file()):
                rel_path = path.relative_to(root).as_posix()
                if path.is_symlink():
                    continue
                resources.append(
                    SkillResourceEntry(
                        path=rel_path,
                        size=path.stat().st_size,
                        kind=resource_dir,
                    )
                )
        return tuple(resources)

    def read_resource(
        self,
        *,
        skill_name: str,
        path: str,
        active_skills: tuple[str, ...],
    ) -> SkillResourceRead:
        _skill, root = self._active_skill_root(skill_name, active_skills)
        rel_path = _validate_resource_path(path)
        resolved = (root / rel_path).resolve(strict=False)
        root_resolved = root.resolve(strict=False)
        if resolved != root_resolved and root_resolved not in resolved.parents:
            raise ColossusError(f"Skill resource path escapes skill root: {path}")
        if not resolved.is_file() or resolved.is_symlink():
            raise ColossusError(f"Skill resource is not a regular file: {path}")
        size = resolved.stat().st_size
        if size > _MAX_RESOURCE_BYTES:
            raise ColossusError(f"Skill resource is too large: {path}")
        if not _is_text_resource(resolved):
            raise ColossusError(f"Skill resource is not a text-safe file: {path}")
        return SkillResourceRead(
            path=rel_path,
            size=size,
            content=resolved.read_text(encoding="utf-8"),
        )

    def _active_skill_root(
        self,
        skill_name: str,
        active_skills: tuple[str, ...],
    ) -> tuple[Skill, Path]:
        if skill_name not in set(active_skills):
            raise ColossusError(f"Skill is not active for this turn: {skill_name}")
        skill = self._resolver.get_skill(skill_name)
        if skill is None:
            raise ColossusError(f"Unknown skill: {skill_name}")
        if skill.resource_root is None:
            raise ColossusError(f"Skill has no readable resource root: {skill_name}")
        root = Path(skill.resource_root)
        if not root.exists() or not root.is_dir():
            raise ColossusError(f"Skill resource root is unavailable: {skill_name}")
        return skill, root


def extract_skill_mentions(
    text: str,
    *,
    available_names: Iterable[str] | None = (),
) -> tuple[str, ...]:
    available = set(available_names) if available_names is not None else None
    names: list[str] = []
    for match in _SKILL_MENTION_RE.finditer(text):
        canonical = match.group("canonical")
        shorthand = match.group("shorthand")
        if canonical is not None:
            names.append(canonical)
        elif shorthand is not None and (available is None or shorthand in available):
            names.append(shorthand)
    return _dedupe(tuple(names))


def _format_skill_context(
    available: tuple[Skill, ...],
    active: tuple[Skill, ...],
) -> str:
    sections = ["[Available skills]"]
    if available:
        sections.append("Mention @skill:name to activate full instructions for this turn.")
        sections.extend(
            f"- {skill.manifest.name} v{skill.manifest.version}: "
            f"{skill.manifest.description}"
            for skill in available
        )
    else:
        sections.append("No enabled skills are available.")
    if active:
        sections.append("")
        sections.append("[Active skills]")
        sections.append("Follow these skill instructions for this turn.")
        for skill in active:
            sections.append("")
            sections.append(f"## {skill.manifest.name} v{skill.manifest.version}")
            sections.append(skill.instructions.strip())
    return "\n".join(sections)


def _format_missing_required_tools(
    missing: tuple[str, ...],
    tool_names: set[str],
) -> str:
    values: list[str] = []
    for name in missing:
        suggestion = name.replace("_", ".")
        if suggestion in tool_names:
            values.append(f"{name} (did you mean {suggestion}?)")
            continue
        values.append(name)
    return ", ".join(values)


def _dedupe(names: tuple[str, ...]) -> tuple[str, ...]:
    seen: set[str] = set()
    deduped: list[str] = []
    for name in names:
        normalized = name.strip()
        if normalized and normalized not in seen:
            seen.add(normalized)
            deduped.append(normalized)
    return tuple(deduped)


def _validate_resource_path(value: str) -> str:
    path = PurePosixPath(value.strip())
    if not path.parts or path.is_absolute() or ".." in path.parts:
        raise ColossusError(f"Invalid skill resource path: {value}")
    if path.parts[0] not in _RESOURCE_DIRS:
        raise ColossusError(
            "Skill resources must live under references, scripts, assets, examples, or tests."
        )
    return path.as_posix()


def _is_text_resource(path: Path) -> bool:
    if path.suffix.lower() in _TEXT_RESOURCE_SUFFIXES:
        return True
    data = path.read_bytes()
    return b"\x00" not in data
