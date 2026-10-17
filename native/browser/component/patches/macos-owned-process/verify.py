#!/usr/bin/env python3
"""Check the exact pinned source and patch, without fetching or building CEF."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--chromium-src", required=True, type=Path)
    parser.add_argument("--apply", action="store_true", help="After verification, apply the patch and stage its fixed client overlay; never build CEF.")
    arguments = parser.parse_args()
    package = Path(__file__).resolve().parent
    manifest = json.loads((package / "manifest.json").read_text())
    patch = package / "chromium.patch"
    if hashlib.sha256(patch.read_bytes()).hexdigest() != manifest["patch_sha256"]:
        raise SystemExit("patch digest differs from the locked manifest")
    for name, expected in manifest["source_sha256"].items():
        target = arguments.chromium_src / name
        if target.is_symlink() or hashlib.sha256(target.read_bytes()).hexdigest() != expected:
            raise SystemExit(f"upstream source differs from exact pinned revision: {name}")
    overlay = {}
    source_root = package.parents[2] / "process-broker"
    for name, destination in manifest["overlay_sources"].items():
        source = source_root / name
        data = source.read_bytes()
        if source.is_symlink() or hashlib.sha256(data).hexdigest() != manifest["overlay_sha256"][name]:
            raise SystemExit(f"client overlay differs from locked source: {name}")
        if name == "client.cc":
            data = data.replace(b'#include "client.h"', b'#include "base/process/colossus_owned_process_client.h"', 1)
        target = arguments.chromium_src / destination
        if target.exists() or target.is_symlink():
            raise SystemExit(f"refusing to replace an existing client overlay: {destination}")
        overlay[target] = data
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=arguments.chromium_src, check=True)
    if arguments.apply:
        subprocess.run(["git", "apply", str(patch)], cwd=arguments.chromium_src, check=True)
        for target, data in overlay.items():
            with target.open("xb") as output:
                output.write(data)
    print("Pinned Chromium source hashes and patch applicability passed; no CEF build or native acceptance.")


if __name__ == "__main__":
    main()
