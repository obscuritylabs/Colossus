"""Skill authoring helpers shared by CLI, REPL, and tools."""

import hashlib
import json
import re
import shutil
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

from colossus.application.skill_loader import (
    load_skill_from_directory,
    parse_skill_frontmatter,
)
from colossus.domain.errors import ColossusError
from colossus.domain.skills import SkillManifest

_SKILL_NAME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9_.-]*$")
_MAX_SKILL_INSTRUCTIONS_CHARS = 60_000
_MAX_SKILL_MD_CHARS = 80_000
_MAX_SKILL_FILE_BYTES = 80_000
_MAX_INSPECT_FILES = 200
_RESOURCE_DIRS = frozenset({"references", "scripts", "assets", "examples", "tests"})
_TEXT_EXTENSIONS = frozenset(
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


@dataclass(frozen=True)
class SkillFileInfo:
    path: str
    size: int
    sha256: str


@dataclass(frozen=True)
class SkillInspectResult:
    name: str
    path: Path
    files: tuple[SkillFileInfo, ...]
    truncated: bool
    validation: SkillValidationResult


@dataclass(frozen=True)
class SkillReadResult:
    name: str
    path: str
    size: int
    sha256: str
    content: str


@dataclass(frozen=True)
class SkillWriteResult:
    name: str
    path: str
    size: int
    sha256: str
    mode: str
    validation: SkillValidationResult


@dataclass(frozen=True)
class SkillInstallResult:
    name: str
    source_path: Path
    target_path: Path
    files: tuple[SkillFileInfo, ...]
    validation: SkillValidationResult


class SkillAuthoringService:
    def __init__(
        self,
        user_skill_root: Path,
        *,
        workspace_skill_root: Path | None = None,
        global_skill_root: Path | None = None,
    ) -> None:
        self._user_skill_root = user_skill_root
        self._workspace_skill_root = workspace_skill_root
        self._global_skill_root = global_skill_root or Path.home() / ".agents" / "skills"

    @property
    def user_skill_root(self) -> Path:
        return self._user_skill_root

    @property
    def workspace_skill_root(self) -> Path | None:
        return self._workspace_skill_root

    @property
    def global_skill_root(self) -> Path:
        return self._global_skill_root

    def scaffold(
        self,
        name: str,
        *,
        description: str | None = None,
        instructions: str | None = None,
        triggers: Sequence[str] | None = None,
        required_tools: Sequence[str] | None = None,
        permissions: Sequence[str] | None = None,
        offline_compatible: bool = True,
        resources: Sequence[str] | None = None,
        agent_compatible: bool = False,
        parent: Path | None = None,
        overwrite: bool = False,
    ) -> SkillScaffoldResult:
        normalized = _normalize_skill_name(name)
        target_root = parent or self._workspace_skill_root or self._user_skill_root
        path = (target_root / normalized).resolve(strict=False)
        if path.exists() and not overwrite:
            raise ColossusError(
                f"Skill directory already exists: {path}. Use --force to overwrite."
            )
        manifest = SkillManifest(
            name=normalized,
            version="0.1.0",
            description=description or f"{normalized} skill.",
            triggers=_normalize_string_list(
                triggers,
                fallback=_triggers_from_name(normalized),
            ),
            required_tools=_normalize_string_list(required_tools),
            permissions=_normalize_string_list(permissions),
            offline_compatible=offline_compatible,
        )
        skill_text = _normalize_skill_instructions(instructions) or _skill_template(
            normalized,
            manifest.description,
            agent_compatible=agent_compatible,
        )
        path.mkdir(parents=True, exist_ok=True)
        (path / "manifest.json").write_text(
            f"{json.dumps(manifest.model_dump(mode='json'), indent=2)}\n",
            encoding="utf-8",
        )
        (path / "SKILL.md").write_text(skill_text, encoding="utf-8")
        for resource_dir in _normalize_resource_dirs(resources):
            (path / resource_dir).mkdir(exist_ok=True)
        return SkillScaffoldResult(name=normalized, path=path, manifest=manifest)

    def scaffold_user_skill(
        self,
        name: str,
        *,
        description: str | None = None,
        instructions: str | None = None,
        triggers: Sequence[str] | None = None,
        required_tools: Sequence[str] | None = None,
        permissions: Sequence[str] | None = None,
        offline_compatible: bool = True,
        resources: Sequence[str] | None = None,
        agent_compatible: bool = False,
        overwrite: bool = False,
    ) -> SkillScaffoldResult:
        return self.scaffold(
            name,
            description=description,
            instructions=instructions,
            triggers=triggers,
            required_tools=required_tools,
            permissions=permissions,
            offline_compatible=offline_compatible,
            resources=resources,
            agent_compatible=agent_compatible,
            parent=self._user_skill_root,
            overwrite=overwrite,
        )

    def scaffold_workspace_skill(
        self,
        name: str,
        *,
        description: str | None = None,
        instructions: str | None = None,
        triggers: Sequence[str] | None = None,
        required_tools: Sequence[str] | None = None,
        permissions: Sequence[str] | None = None,
        offline_compatible: bool = True,
        resources: Sequence[str] | None = None,
        agent_compatible: bool = False,
        overwrite: bool = False,
    ) -> SkillScaffoldResult:
        if self._workspace_skill_root is None:
            raise ColossusError("Workspace skill root is not configured.")
        return self.scaffold(
            name,
            description=description,
            instructions=instructions,
            triggers=triggers,
            required_tools=required_tools,
            permissions=permissions,
            offline_compatible=offline_compatible,
            resources=resources,
            agent_compatible=agent_compatible,
            parent=self._workspace_skill_root,
            overwrite=overwrite,
        )

    def install_skill(self, source_path: Path, *, overwrite: bool = False) -> SkillInstallResult:
        source = source_path.resolve(strict=False)
        validation = self.validate(source)
        if not validation.valid or validation.manifest is None:
            raise ColossusError(
                "Cannot install invalid skill: " + "; ".join(validation.errors)
            )
        _reject_symlinks(source)
        root = self._global_skill_root.resolve(strict=False)
        target = (root / validation.manifest.name).resolve(strict=False)
        if not _is_relative_to(target, root):
            raise ColossusError("Install target escapes the configured global skill directory.")
        if target.exists() and not overwrite:
            raise ColossusError(
                f"Global skill already exists: {validation.manifest.name}. "
                "Use --force to overwrite."
            )
        root.mkdir(parents=True, exist_ok=True)
        if target.exists():
            if target.is_symlink():
                raise ColossusError("Install target symlinks are not allowed.")
            if not target.is_dir():
                raise ColossusError("Install target is not a directory.")
            shutil.rmtree(target)
        shutil.copytree(source, target)
        installed_validation = self.validate(target)
        if not installed_validation.valid:
            raise ColossusError(
                "Installed skill failed validation: " + "; ".join(installed_validation.errors)
            )
        return SkillInstallResult(
            name=validation.manifest.name,
            source_path=source,
            target_path=target,
            files=_skill_file_infos(target),
            validation=installed_validation,
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
        skill_path = _skill_markdown_path(resolved)
        try:
            loaded = load_skill_from_directory(resolved, source=str(resolved))
            if loaded is None:
                errors.append("SKILL.md is missing.")
            else:
                manifest = loaded.manifest
        except (ColossusError, ValueError) as exc:
            errors.append(str(exc))
        if manifest is not None and not _SKILL_NAME_RE.fullmatch(manifest.name):
            errors.append(
                "manifest.json name must start with a letter and contain only "
                "letters, numbers, dots, underscores, or hyphens."
            )
        if skill_path is None:
            if "SKILL.md is missing." not in errors:
                errors.append("SKILL.md is missing.")
        else:
            raw_skill_text = skill_path.read_text(encoding="utf-8")
            _frontmatter, skill_body = parse_skill_frontmatter(raw_skill_text)
            skill_text = skill_body.strip()
            if not skill_text:
                errors.append("SKILL.md is empty.")
            if len(raw_skill_text) > _MAX_SKILL_MD_CHARS:
                errors.append(f"SKILL.md is larger than {_MAX_SKILL_MD_CHARS} characters.")
        errors.extend(_validate_skill_resources(resolved))
        return SkillValidationResult(
            path=resolved,
            valid=not errors,
            manifest=manifest,
            errors=tuple(errors),
        )

    def validate_user_skill(self, name: str) -> SkillValidationResult:
        normalized = _normalize_skill_name(name)
        return self.validate(self._user_skill_root / normalized)

    def inspect_user_skill(self, name: str) -> SkillInspectResult:
        normalized = _normalize_skill_name(name)
        skill_dir = self._existing_user_skill_dir(normalized)
        files: list[SkillFileInfo] = []
        truncated = False
        for item in sorted(skill_dir.rglob("*")):
            if len(files) >= _MAX_INSPECT_FILES:
                truncated = True
                break
            if item.is_symlink() or not item.is_file():
                continue
            rel_path = item.relative_to(skill_dir).as_posix()
            files.append(
                SkillFileInfo(
                    path=rel_path,
                    size=item.stat().st_size,
                    sha256=_sha256_file(item),
                )
            )
        return SkillInspectResult(
            name=normalized,
            path=skill_dir,
            files=tuple(files),
            truncated=truncated,
            validation=self.validate(skill_dir),
        )

    def read_user_skill_file(self, name: str, path: str = "SKILL.md") -> SkillReadResult:
        normalized = _normalize_skill_name(name)
        skill_dir = self._existing_user_skill_dir(normalized)
        rel_path = _normalize_authoring_relative_path(path)
        target = _resolve_authoring_file(skill_dir, rel_path)
        if not target.exists():
            raise ColossusError(f"Skill file does not exist: {rel_path.as_posix()}.")
        if target.is_symlink():
            raise ColossusError(f"Skill file symlinks are not allowed: {rel_path.as_posix()}.")
        if not target.is_file():
            raise ColossusError(f"Skill path is not a file: {rel_path.as_posix()}.")
        size = target.stat().st_size
        if size > _MAX_SKILL_FILE_BYTES:
            raise ColossusError(
                f"Skill file is larger than {_MAX_SKILL_FILE_BYTES} bytes: "
                f"{rel_path.as_posix()}."
            )
        try:
            content = target.read_text(encoding="utf-8")
        except UnicodeDecodeError as exc:
            raise ColossusError(
                f"Skill file is not UTF-8 text: {rel_path.as_posix()}."
            ) from exc
        return SkillReadResult(
            name=normalized,
            path=rel_path.as_posix(),
            size=size,
            sha256=_sha256_text(content),
            content=content,
        )

    def write_user_skill_file(
        self,
        name: str,
        path: str,
        content: str,
        *,
        mode: str = "overwrite",
        expected_sha256: str | None = None,
    ) -> SkillWriteResult:
        normalized = _normalize_skill_name(name)
        skill_dir = self._existing_user_skill_dir(normalized)
        rel_path = _normalize_authoring_relative_path(path)
        target = _resolve_authoring_file(skill_dir, rel_path)
        _validate_authoring_content(rel_path, content, normalized)
        if mode not in {"create", "overwrite"}:
            raise ColossusError("Skill write mode must be create or overwrite.")
        if target.exists() and target.is_symlink():
            raise ColossusError(f"Skill file symlinks are not allowed: {rel_path.as_posix()}.")
        if target.exists() and not target.is_file():
            raise ColossusError(f"Skill path is not a file: {rel_path.as_posix()}.")
        if mode == "create" and target.exists():
            raise ColossusError(
                f"Skill file already exists: {rel_path.as_posix()}. Use overwrite mode."
            )
        if mode == "overwrite" and target.exists():
            if expected_sha256 is None:
                raise ColossusError(
                    "expected_sha256 is required when overwriting an existing skill file."
                )
            actual_sha256 = _sha256_file(target)
            if expected_sha256 != actual_sha256:
                raise ColossusError(
                    "Skill file changed since it was read; read it again before writing."
                )
        _ensure_safe_authoring_parent(skill_dir, rel_path)
        target.write_text(_normalize_written_text(content), encoding="utf-8")
        written = target.read_text(encoding="utf-8")
        return SkillWriteResult(
            name=normalized,
            path=rel_path.as_posix(),
            size=len(written.encode("utf-8")),
            sha256=_sha256_text(written),
            mode=mode,
            validation=self.validate(skill_dir),
        )

    def _existing_user_skill_dir(self, normalized_name: str) -> Path:
        root = self._user_skill_root.resolve(strict=False)
        path = (root / normalized_name).resolve(strict=False)
        if not _is_relative_to(path, root):
            raise ColossusError("Skill path escapes the configured user skill directory.")
        if not path.exists():
            raise ColossusError(f"User skill does not exist: {normalized_name}.")
        if not path.is_dir():
            raise ColossusError(f"User skill path is not a directory: {normalized_name}.")
        return path


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


def _normalize_string_list(
    values: Sequence[str] | None,
    *,
    fallback: tuple[str, ...] = (),
) -> tuple[str, ...]:
    if values is None:
        return fallback
    normalized: list[str] = []
    for value in values:
        if not isinstance(value, str) or not value.strip():
            raise ColossusError("Skill manifest lists must contain non-empty strings.")
        normalized.append(value.strip())
    return _dedupe(tuple(normalized))


def _normalize_skill_instructions(instructions: str | None) -> str | None:
    if instructions is None:
        return None
    stripped = instructions.strip()
    if not stripped:
        raise ColossusError("Skill instructions must not be empty.")
    if len(stripped) > _MAX_SKILL_INSTRUCTIONS_CHARS:
        raise ColossusError(
            f"Skill instructions must be at most {_MAX_SKILL_INSTRUCTIONS_CHARS} characters."
        )
    return f"{stripped}\n"


def _normalize_resource_dirs(resources: Sequence[str] | None) -> tuple[str, ...]:
    if resources is None:
        return ()
    normalized: list[str] = []
    for resource in resources:
        value = resource.strip()
        if value not in _RESOURCE_DIRS:
            raise ColossusError(
                "Resource directories must be one of: "
                f"{', '.join(sorted(_RESOURCE_DIRS))}."
            )
        normalized.append(value)
    return _dedupe(tuple(normalized))


def _validate_skill_resources(path: Path) -> tuple[str, ...]:
    errors: list[str] = []
    if (
        _file_child_named(path, "SKILL.md") is not None
        and _file_child_named(path, "skill.md") is not None
    ):
        errors.append("Skill directory must not contain both SKILL.md and skill.md.")
    for child in path.iterdir():
        if child.name in _RESOURCE_DIRS:
            if not child.is_dir():
                errors.append(f"{child.name} must be a directory.")
            continue
        if child.name in {"manifest.json", "SKILL.md", "skill.md"}:
            continue
        if child.name.startswith("."):
            continue
        if child.is_dir():
            errors.append(f"Unexpected skill directory: {child.name}.")
    for resource_dir in sorted(_RESOURCE_DIRS):
        root = path / resource_dir
        if not root.exists() or not root.is_dir():
            continue
        for item in root.rglob("*"):
            rel_path = item.relative_to(path).as_posix()
            if item.is_symlink():
                errors.append(f"Resource symlinks are not allowed: {rel_path}.")
                continue
            if not item.is_file():
                continue
            if resource_dir == "scripts":
                continue
            if item.suffix.lower() not in _TEXT_EXTENSIONS:
                errors.append(
                    f"Resource file should be text or live in scripts/: {rel_path}."
                )
    return tuple(errors)


def _normalize_authoring_relative_path(path: str) -> PurePosixPath:
    raw = path.strip()
    if not raw:
        raise ColossusError("Skill file path is required.")
    rel_path = PurePosixPath(raw)
    if rel_path.is_absolute() or ".." in rel_path.parts:
        raise ColossusError("Skill file path must be a safe relative path.")
    if rel_path.name in {"", ".", ".."}:
        raise ColossusError("Skill file path must name a file.")
    if not _is_allowed_authoring_path(rel_path):
        raise ColossusError(
            "Skill file path must be SKILL.md, manifest.json, or a file under "
            f"{', '.join(sorted(_RESOURCE_DIRS))}."
        )
    return rel_path


def _is_allowed_authoring_path(path: PurePosixPath) -> bool:
    if len(path.parts) == 1:
        return path.parts[0] in {"manifest.json", "SKILL.md"}
    return path.parts[0] in _RESOURCE_DIRS


def _resolve_authoring_file(skill_dir: Path, rel_path: PurePosixPath) -> Path:
    target = (skill_dir / Path(*rel_path.parts)).resolve(strict=False)
    if not _is_relative_to(target, skill_dir):
        raise ColossusError("Skill file path escapes the skill directory.")
    return target


def _ensure_safe_authoring_parent(skill_dir: Path, rel_path: PurePosixPath) -> None:
    parent = _resolve_authoring_file(skill_dir, PurePosixPath(*rel_path.parts[:-1]))
    if rel_path.parts[:-1] == ():
        parent = skill_dir
    if not _is_relative_to(parent, skill_dir):
        raise ColossusError("Skill file parent escapes the skill directory.")
    parent.mkdir(parents=True, exist_ok=True)


def _validate_authoring_content(path: PurePosixPath, content: str, skill_name: str) -> None:
    if not isinstance(content, str):
        raise ColossusError("Skill file content must be text.")
    size = len(content.encode("utf-8"))
    if size > _MAX_SKILL_FILE_BYTES:
        raise ColossusError(
            f"Skill file content must be at most {_MAX_SKILL_FILE_BYTES} bytes."
        )
    if path.as_posix() == "SKILL.md" and not content.strip():
        raise ColossusError("SKILL.md must not be empty.")
    if path.as_posix() == "manifest.json":
        try:
            payload = json.loads(content)
            manifest = SkillManifest.model_validate(payload)
        except (json.JSONDecodeError, ValueError) as exc:
            raise ColossusError(f"manifest.json is invalid: {exc}") from exc
        if manifest.name != skill_name:
            raise ColossusError("manifest.json name must match the target skill name.")
    if len(path.parts) > 1 and path.parts[0] != "scripts":
        suffix = Path(path.name).suffix.lower()
        if suffix not in _TEXT_EXTENSIONS:
            raise ColossusError(
                "Resource file should be text or live in scripts/: "
                f"{path.as_posix()}."
            )


def _normalize_written_text(content: str) -> str:
    return content if content.endswith("\n") else f"{content}\n"


def _sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _sha256_text(content: str) -> str:
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def _skill_markdown_path(path: Path) -> Path | None:
    canonical = _file_child_named(path, "SKILL.md")
    protocol = _file_child_named(path, "skill.md")
    if canonical is not None:
        return canonical
    if protocol is not None:
        return protocol
    return None


def _file_child_named(path: Path, name: str) -> Path | None:
    for child in path.iterdir():
        if child.name == name and child.is_file():
            return child
    return None


def _reject_symlinks(path: Path) -> None:
    for item in (path, *path.rglob("*")):
        if item.is_symlink():
            raise ColossusError(f"Skill symlinks are not allowed: {item}")


def _skill_file_infos(skill_dir: Path) -> tuple[SkillFileInfo, ...]:
    files: list[SkillFileInfo] = []
    for item in sorted(skill_dir.rglob("*")):
        if item.is_symlink() or not item.is_file():
            continue
        rel_path = item.relative_to(skill_dir).as_posix()
        files.append(
            SkillFileInfo(path=rel_path, size=item.stat().st_size, sha256=_sha256_file(item))
        )
    return tuple(files)


def _is_relative_to(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


def _skill_template(
    name: str,
    description: str,
    *,
    agent_compatible: bool,
) -> str:
    title = name.replace("-", " ").replace("_", " ").replace(".", " ").title()
    body = (
        f"# {title}\n\n"
        "Use this skill when the user asks for this workflow. Replace this paragraph\n"
        "with the concrete triggers, constraints, and expected outputs for the skill.\n\n"
        "## Workflow\n\n"
        "1. Identify the user's goal, source context, constraints, and required output.\n"
        "2. Inspect only the files or inputs needed for the task.\n"
        "3. Use the smallest relevant tool set and keep changes focused.\n"
        "4. Validate the result before reporting back.\n\n"
        "## Tool And Safety Notes\n\n"
        "- List required model-callable tools in manifest.json only when the skill truly\n"
        "  depends on them.\n"
        "- Keep permissions empty unless the workflow needs an explicit capability.\n"
        "- Do not store secrets, credentials, or hidden policy changes in this skill.\n"
    )
    if not agent_compatible:
        return body
    return (
        "---\n"
        f"name: {name}\n"
        f"description: {description}\n"
        "---\n\n"
        f"{body}"
    )
