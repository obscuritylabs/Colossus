#!/usr/bin/env python3
"""Seal an already signed/inventoried functional preview for compiler embedding.

This descriptor contains no local paths and grants no production containment.
Its digest becomes authoritative only inside the publisher-signed sidecar.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import component

PURPOSE = "colossus-windows-browser-functional-preview"
CEF_VERSION = "154.0.34+g14c5a08+chromium-154.0.8037.98"
ARCHIVE_SHA256 = "c81c34d048d87276f4a18f0b000206710a1709bb67dd658948b87100e1a538da"


def seal(root: Path, output: Path) -> None:
    inventory = component.verify_installed(root)
    if (inventory["platform"] != "windows64"
            or inventory["target"] != "x86_64-pc-windows-msvc"
            or inventory["executable"] != "colossus-native-browser-host.exe"
            or inventory["cef_version"] != CEF_VERSION
            or inventory["archive_sha256"] != ARCHIVE_SHA256
            or inventory["modes"] != {"desktop": False, "headless": False}):
        raise component.ComponentError("only the pinned false-mode Windows host can enter this preview")
    manifest = root / "browser-component.json"
    component.digest_file(manifest)
    descriptor = {
        "schema_version": 1, "purpose": PURPOSE, "target": "x86_64-pc-windows-msvc",
        "component_path": "browser/component",
        "component_manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
        "cef_version": CEF_VERSION, "source_archive_sha256": ARCHIVE_SHA256,
        "production_acceptance": False,
        "capabilities": {"embedded": True, "headless": False, "restrictive_egress": False,
                         "private_ca_trust": False, "client_identities": False},
    }
    encoded = json.dumps(descriptor, separators=(",", ":")).encode("ascii")
    # No overwrite: a signing mutation requires a fresh stage and seal.
    with output.open("xb") as stream:
        stream.write(encoded)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        seal(arguments.component, arguments.output)
        return 0
    except (component.ComponentError, OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Windows functional-preview seal: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
