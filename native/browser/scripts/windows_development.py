#!/usr/bin/env python3
"""Create one private, explicit compiler selection for non-admin Windows development."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path

import component
from pki_fixture_private import private_directory, private_write


def prepare(root: Path, browser: Path) -> Path:
    if os.name != "nt" or not root.is_absolute() or not browser.is_absolute():
        raise ValueError("native Windows absolute development allocations required")
    inventory = component.verify_installed(browser)
    if inventory["platform"] != "windows64" or inventory["modes"] != {
        "desktop": False, "headless": False
    }:
        raise ValueError("only the verified false-mode Windows source component is admitted")
    manifest = (browser / "browser-component.json").read_bytes()
    private_directory(root)
    state = root / "state"
    private_directory(state)
    selection = root / "windows-browser-development.json"
    private_write(selection, json.dumps({
        "component_root": str(browser),
        "state_root": str(state),
        "manifest_sha256": hashlib.sha256(manifest).hexdigest(),
    }, separators=(",", ":")).encode("utf-8"))
    return selection


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(prepare(args.destination, args.component))
        return 0
    except (ValueError, OSError) as error:
        parser.exit(1, f"Windows browser development: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
