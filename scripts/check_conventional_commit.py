#!/usr/bin/env python3
"""Validate a commit message against the Conventional Commits shape."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ALLOWED_TYPES = (
    "build",
    "chore",
    "ci",
    "docs",
    "feat",
    "fix",
    "perf",
    "refactor",
    "revert",
    "security",
    "style",
    "test",
)

CONVENTIONAL_HEADER = re.compile(
    rf"^({'|'.join(ALLOWED_TYPES)})(\([a-z0-9._-]+\))?!?: [^\s].+$"
)
IGNORED_HEADERS = (
    "Merge ",
    "Revert ",
    "fixup! ",
    "squash! ",
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate Conventional Commit messages.",
    )
    parser.add_argument("message_file", nargs="?", type=Path)
    parser.add_argument("--stdin", action="store_true", help="Read a message from stdin.")
    parser.add_argument("--range", dest="commit_range", help="Validate commits in a git range.")
    args = parser.parse_args(argv)
    source_count = sum(
        (
            args.message_file is not None,
            args.stdin,
            args.commit_range is not None,
        )
    )
    if source_count != 1:
        parser.error("provide exactly one of message_file, --stdin, or --range")

    if args.stdin:
        messages = [sys.stdin.read()]
    elif args.commit_range:
        messages = _messages_from_range(args.commit_range)
    else:
        messages = [args.message_file.read_text(encoding="utf-8")]

    failures = [message for message in messages if not is_valid_message(message)]
    if not failures:
        return 0

    for message in failures:
        _print_error(_subject_line(message))
    return 1


def is_valid_message(message: str) -> bool:
    subject = _subject_line(message)
    if not subject:
        return False
    if subject.startswith(IGNORED_HEADERS):
        return True
    return bool(CONVENTIONAL_HEADER.fullmatch(subject))


def _messages_from_range(commit_range: str) -> list[str]:
    result = subprocess.run(
        ["git", "log", "--format=%B%x00", commit_range],
        check=True,
        capture_output=True,
        text=True,
    )
    return [message for message in result.stdout.split("\x00") if message.strip()]


def _subject_line(message: str) -> str:
    for line in message.splitlines():
        stripped = line.strip()
        if stripped and not stripped.startswith("#"):
            return stripped
    return ""


def _print_error(subject: str) -> None:
    sys.stderr.write("Commit message is not Conventional Commits compliant.\n")
    sys.stderr.write(f"Found: {subject or '<empty>'}\n")
    sys.stderr.write("Expected: <type>[optional scope][!]: <description>\n")
    sys.stderr.write(f"Allowed types: {', '.join(ALLOWED_TYPES)}\n")
    sys.stderr.write("Examples: feat(repl): add themes | fix: handle denied approvals\n")


if __name__ == "__main__":
    raise SystemExit(main())
