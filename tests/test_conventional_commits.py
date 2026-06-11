from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

SCRIPT_PATH = Path(__file__).resolve().parents[1] / "scripts" / "check_conventional_commit.py"
SPEC = importlib.util.spec_from_file_location("check_conventional_commit", SCRIPT_PATH)
assert SPEC is not None
assert SPEC.loader is not None
check_conventional_commit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_conventional_commit)


@pytest.mark.parametrize(
    "message",
    [
        "feat: add repl themes",
        "fix(repl): clear approved prompt",
        "security!: tighten approval policy",
        "docs(shell.run): explain structured argv",
    ],
)
def test_conventional_commit_validator_accepts_valid_messages(message: str) -> None:
    assert check_conventional_commit.is_valid_message(message)


@pytest.mark.parametrize(
    "message",
    [
        "",
        "Update docs",
        "feature: add repl themes",
        "fix: ",
        "fix(repl):",
        "Fix: capitalized type",
    ],
)
def test_conventional_commit_validator_rejects_invalid_messages(message: str) -> None:
    assert not check_conventional_commit.is_valid_message(message)


@pytest.mark.parametrize(
    "message",
    [
        "Merge branch 'main' into feature",
        'Revert "feat: add repl themes"',
        "fixup! fix: clear prompt",
        "squash! feat: add prompt",
    ],
)
def test_conventional_commit_validator_ignores_git_generated_messages(message: str) -> None:
    assert check_conventional_commit.is_valid_message(message)


def test_conventional_commit_validator_reads_message_file(tmp_path: Path) -> None:
    message_file = tmp_path / "COMMIT_EDITMSG"
    message_file.write_text(
        "\n# Please enter the commit message\nfix: handle approval prompt\n",
        encoding="utf-8",
    )

    assert check_conventional_commit.main([str(message_file)]) == 0
