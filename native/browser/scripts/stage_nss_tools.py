#!/usr/bin/env python3
"""Explicit builder/installer: stage already installed, reviewed NSS utilities.

No browser/runtime download or package installation. Install the exact locked OS
package via the builder's reviewed package acquisition flow before invoking this.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat

LOCK = Path(__file__).resolve().parent.parent / "nss-tools.lock.json"


def stage(destination: Path) -> dict:
    lock = json.loads(LOCK.read_text())
    if destination.exists() or not destination.is_absolute():
        raise ValueError("NSS tools require a new absolute installer destination")
    verified = {}
    for name, artifact in lock["artifacts"].items():
        source = Path(artifact["source"])
        metadata = source.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_mode & 0o022:
            raise ValueError("NSS source artifact is mutable or indirect")
        if metadata.st_size > 16 * 1024 * 1024:
            raise ValueError("NSS source artifact exceeds bounds")
        if hashlib.sha256(source.read_bytes()).hexdigest() != artifact["sha256"]:
            raise ValueError("Installed NSS tools/dependencies do not match the reviewed lock")
        verified[name] = source
    destination.mkdir(mode=0o700)
    try:
        for name, source in verified.items():
            target = destination / name
            # A new independent inode: no hardlink to a personal or OS store.
            shutil.copyfile(source, target)
            if hashlib.sha256(target.read_bytes()).hexdigest() != lock["artifacts"][name]["sha256"]:
                raise ValueError("NSS artifact changed while staging")
            target.chmod(0o555 if name in ("certutil", "pk12util") else 0o444)
        inventory = {"schema_version": 1, "platform": lock["platform"],
                     "nss_package_version": lock["nss_package_version"],
                     "nspr_package_version": lock["nspr_package_version"],
                     "artifacts": {name: value["sha256"] for name, value in lock["artifacts"].items()},
                     "production_acceptance": False}
        receipt = destination / "nss-tools-inventory.json"
        receipt.write_text(json.dumps(inventory, indent=2) + "\n")
        receipt.chmod(0o444)
        destination.chmod(0o555)
        return inventory
    except BaseException:
        destination.chmod(0o700)
        shutil.rmtree(destination)
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    arguments = parser.parse_args()
    try:
        print(json.dumps(stage(arguments.destination), sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError):
        print("NSS installer staging failed; install the reviewed locked OS tooling first", file=__import__("sys").stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
