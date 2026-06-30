from pathlib import Path

import pytest

from colossus.adapters.workspace import Workspace
from colossus.domain.errors import ToolExecutionError


def test_workspace_denies_parent_traversal(tmp_path: Path) -> None:
    workspace = Workspace(tmp_path / "root")

    with pytest.raises(ToolExecutionError):
        workspace.resolve("../outside.txt")


def test_workspace_denies_absolute_path_outside_root(tmp_path: Path) -> None:
    root = tmp_path / "root"
    outside = tmp_path / "outside.txt"
    workspace = Workspace(root)

    with pytest.raises(ToolExecutionError):
        workspace.resolve(str(outside))


def test_workspace_denies_symlink_escape(tmp_path: Path) -> None:
    root = tmp_path / "root"
    root.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()
    (root / "escape").symlink_to(outside)
    workspace = Workspace(root)

    with pytest.raises(ToolExecutionError):
        workspace.resolve("escape/file.txt")


def test_workspace_denies_control_directories(tmp_path: Path) -> None:
    root = tmp_path / "root"
    workspace = Workspace(root)

    with pytest.raises(ToolExecutionError):
        workspace.resolve(".git/config")

    with pytest.raises(ToolExecutionError):
        workspace.resolve(".colossus/state.json")


def test_workspace_allows_agents_directory(tmp_path: Path) -> None:
    root = tmp_path / "root"
    workspace = Workspace(root)

    assert workspace.resolve(".agents/skills/demo/SKILL.md") == (
        root / ".agents" / "skills" / "demo" / "SKILL.md"
    ).resolve(strict=False)
