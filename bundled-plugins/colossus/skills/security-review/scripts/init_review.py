#!/usr/bin/env python3
"""Initialize private security-review notes without modifying source or Git config."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import tempfile


def git_ignores_notes(workspace: Path, review_id: str) -> bool:
    git = shutil.which("git")
    if git is None:
        return False
    try:
        result = subprocess.run(
            [
                git,
                "-c", "core.fsmonitor=false",
                "check-ignore", "--quiet", "--no-index", "--",
                f".local/security-reviews/{review_id}/brief.md",
            ],
            cwd=workspace,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    return result.returncode == 0


def safe_local_parents(workspace: Path) -> bool:
    for path in (workspace / ".local", workspace / ".local/security-reviews"):
        if path.is_symlink() or path.resolve() != path:
            return False
        if path.exists() and not path.is_dir():
            return False
    return True


def new_file(path: Path, content: str) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(path, flags, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as output:
        output.write(content)


def initialize(workspace: Path, review_id: str) -> dict:
    if re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", review_id) is None:
        raise ValueError("review-id must be 1-64 lowercase letters, digits, or hyphens")
    workspace = workspace.resolve(strict=True)
    if not workspace.is_dir():
        raise ValueError("workspace must be an existing directory")

    review_dir = None
    reason = "checkout-local notes are not confirmed ignored or their parents are unsafe"
    if git_ignores_notes(workspace, review_id) and safe_local_parents(workspace):
        try:
            (workspace / ".local").mkdir(mode=0o700, exist_ok=True)
            root = workspace / ".local/security-reviews"
            root.mkdir(mode=0o700, exist_ok=True)
            if not safe_local_parents(workspace):
                raise OSError("scratch parent changed during initialization")
            candidate = root / review_id
            candidate.mkdir(mode=0o700)
            review_dir = candidate
        except FileExistsError as error:
            raise ValueError("review directory already exists; resume it or choose a new ID") from error
        except OSError:
            reason = "checkout-local scratch could not be created; using OS temporary storage"

    storage = "workspace"
    if review_dir is None:
        review_dir = Path(tempfile.mkdtemp(prefix=f"colossus-security-review-{review_id}-"))
        storage = "temporary"

    templates = {
        "brief.md": (
            "# Review brief\n\n"
            f"Review ID: {review_id}\n"
            f"Storage checkout: {json.dumps(str(workspace))}\n"
            "Reviewed target, revision, and dirty state: not yet recorded\n"
            "Request and in-scope paths/diff: not yet recorded\n"
            "Deployment, supplied model, and assumptions: not yet recorded\n"
            "Exclusions, budgets, tools, and execution limits: not yet recorded\n"
        ),
        "threat-model.md": (
            "# Threat model\n\nStatus: not started\n\n"
            "Record source-backed actors, assets, boundaries, effective controls/resources,\n"
            "assumptions, and investigation packets. Scenarios are not verified findings.\n"
        ),
        "checkpoint.md": (
            "# Review checkpoint\n\nStatus: initialized; source review not started\n\n"
            "Next: record scope and snapshot, map boundaries, and form bounded packets.\n"
            "Record packet/job IDs and states, candidate decisions, proof gaps, and pointers.\n"
        ),
        "coverage.json": json.dumps({"status": "not_started", "surfaces": []}, indent=2) + "\n",
        "findings.json": json.dumps({"candidates": []}, indent=2) + "\n",
    }
    for name, content in templates.items():
        new_file(review_dir / name, content)
    (review_dir / "workers").mkdir(mode=0o700)
    return {
        "status": "initialized",
        "review_id": review_id,
        "review_dir": str(review_dir),
        "storage": storage,
        "files": list(templates),
        "notice": reason if storage == "temporary" else "checkout-local notes are Git-ignored",
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--review-id", help="optional unique lowercase alphanumeric/hyphen ID")
    arguments = parser.parse_args()
    review_id = arguments.review_id or (
        datetime.now(timezone.utc).strftime("%Y%m%dt%H%M%Sz") + "-" + secrets.token_hex(4)
    )
    try:
        result = initialize(arguments.workspace, review_id)
    except (OSError, ValueError) as error:
        parser.exit(1, f"Cannot initialize review notes: {error}\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
