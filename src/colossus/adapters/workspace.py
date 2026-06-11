"""Workspace-safe path handling."""

from pathlib import Path

from colossus.domain.errors import ToolExecutionError

DENIED_CONTROL_DIRS = frozenset({".git", ".hg", ".svn", ".colossus"})


class Workspace:
    def __init__(self, root: Path) -> None:
        self.root = root.resolve()

    def resolve(self, value: str | None = None) -> Path:
        raw = value or "."
        candidate = Path(raw)
        if candidate.is_absolute():
            resolved = candidate.resolve(strict=False)
        else:
            resolved = (self.root / candidate).resolve(strict=False)
        self._ensure_inside_root(resolved)
        self._ensure_allowed_components(resolved)
        return resolved

    def relative(self, path: Path) -> str:
        resolved = path.resolve(strict=False)
        self._ensure_inside_root(resolved)
        return resolved.relative_to(self.root).as_posix() or "."

    def _ensure_inside_root(self, path: Path) -> None:
        try:
            path.relative_to(self.root)
        except ValueError as exc:
            raise ToolExecutionError(f"Path escapes workspace root: {path}") from exc

    def _ensure_allowed_components(self, path: Path) -> None:
        relative = path.relative_to(self.root)
        denied = DENIED_CONTROL_DIRS.intersection(relative.parts)
        if denied:
            raise ToolExecutionError(f"Path enters denied control directory: {sorted(denied)[0]}")
