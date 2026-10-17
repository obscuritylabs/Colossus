#!/usr/bin/env python3
"""Build or explicitly run the reviewed unsupported macOS owned-profile fixture.

Build never executes the fixture or calls Keychain APIs. Run creates an owned file Keychain and
captures only metadata/categorical evidence in a fresh private artifact directory.
It never starts Chromium, adopts old state, or changes trust/default/search lists.
An unknown result preserves all physical artifacts and evidence for reconciliation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import stat
import subprocess
import sys

from pki_fixture_private import private_directory, private_write


class FixtureError(Exception):
    """Categorical fixture failure; never contains private native output."""


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def private_file(path: Path, *, empty: bool = False) -> None:
    value = path.lstat()
    if (not stat.S_ISREG(value.st_mode) or value.st_uid != os.geteuid()
            or value.st_nlink != 1 or stat.S_IMODE(value.st_mode) != 0o600
            or (empty and value.st_size != 0)):
        raise FixtureError("private fixture evidence is not fresh")


def private_root(path: Path) -> None:
    value = path.lstat()
    if (not stat.S_ISDIR(value.st_mode) or value.st_uid != os.geteuid()
            or stat.S_IMODE(value.st_mode) != 0o700
            or path != path.resolve(strict=True)):
        raise FixtureError("fixture root is not canonical and private")


def durable_write(path: Path, data: bytes) -> None:
    """Publish one new receipt and flush its file and directory before effects."""
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                         os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY |
                        os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def checked(command: list[str]) -> dict:
    result = subprocess.run(command, capture_output=True, text=True, timeout=60)
    if result.returncode:
        raise FixtureError("fixture compilation/signature verification failed")
    # Build diagnostics are retained privately, never displayed automatically.
    return {"command": command, "stdout": result.stdout, "stderr": result.stderr}


def build(native_build: Path, artifacts: Path) -> None:
    repository = Path(__file__).resolve().parents[3]
    native_build = native_build.resolve(strict=True)
    marker = json.loads((native_build / "profile-crypto-development-build.json").read_text())
    if (marker.get("schema_version") != 1 or marker.get("development_only") is not True
            or marker.get("build_type") != "Debug"
            or marker.get("production_accepted") is not False
            or marker.get("api") != "unsupported-dyld-security-spi"):
        raise FixtureError("supply the explicit unsupported Debug profile adapter")
    if artifacts.exists() or artifacts.is_symlink():
        raise FixtureError("existing fixture artifacts are preserved")
    if artifacts.parent != artifacts.parent.resolve(strict=True):
        raise FixtureError("fixture parent must be canonical")
    private_directory(artifacts)
    private_root(artifacts)
    parent = artifacts / "owned-state"
    private_directory(parent)
    for name in ("store-proof.stdout.json", "store-proof.stderr.log"):
        private_write(artifacts / name, b"")
    source = artifacts / "reviewed-fixture.mm"
    private_write(source, (repository / "native/browser/mac/profile_crypto_fixture.mm").read_bytes())
    adapter_source = artifacts / "profile_crypto.mm"
    adapter_header = artifacts / "profile_crypto.h"
    private_write(adapter_source, (repository / "native/browser/mac/profile_crypto.mm").read_bytes())
    private_write(adapter_header, (repository / "native/browser/mac/profile_crypto.h").read_bytes())
    app = artifacts / "Scoped Profile Crypto Fixture.app"
    contents = app / "Contents"
    macos = contents / "MacOS"
    libraries = contents / "Frameworks"
    for directory in (app, contents, macos, libraries):
        private_directory(directory)
    library = libraries / "libcolossus_mac_profile_crypto.dylib"
    binary = macos / "scoped-profile-crypto-fixture"
    architecture = platform.machine()
    if architecture not in ("arm64", "x86_64"):
        raise FixtureError("unsupported fixture architecture")
    commands = [
        # Compile the saved reviewed snapshots, never relabel a supplied
        # prebuilt dylib with the current repository's source hash.
        ["/usr/bin/clang++", "-std=c++20", "-O0", "-g", "-Wall", "-Wextra", "-Werror",
         "-arch", architecture, "-dynamiclib", "-fvisibility=hidden", "-fvisibility-inlines-hidden",
         "-DCOLOSSUS_MAC_PROFILE_CRYPTO_DEVELOPMENT=1", str(adapter_source), "-I" + str(artifacts),
         "-framework", "Security", "-framework", "CoreFoundation",
         "-Wl,-install_name,@rpath/libcolossus_mac_profile_crypto.dylib", "-o", str(library)],
        ["/usr/bin/clang++", "-std=c++20", "-O0", "-g", "-Wall", "-Wextra", "-Werror",
         "-arch", architecture, str(source), "-I" + str(artifacts),
         "-L" + str(libraries), "-lcolossus_mac_profile_crypto", "-framework", "Security",
         "-framework", "CoreFoundation", "-Wl,-rpath,@executable_path/../Frameworks",
         "-o", str(binary)],
        ["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", str(library)],
    ]
    private_write(contents / "Info.plist", plistlib.dumps({
        "CFBundleIdentifier": "com.obscuritylabs.colossus.profile-crypto-fixture",
        "CFBundleExecutable": binary.name, "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1", "CFBundleShortVersionString": "0.0.1",
    }))
    commands += [
        ["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", "--options", "runtime",
         "--entitlements", str(repository / "native/browser/mac/development.entitlements.plist"), str(app)],
        ["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)],
        ["/usr/bin/otool", "-L", str(binary)],
    ]
    evidence = [checked(command) for command in commands]
    relative_files = [path.relative_to(artifacts) for path in
                      (source, adapter_source, adapter_header, binary, library)]
    private_write(artifacts / "build-evidence.json", (json.dumps({
        "schema_version": 1, "executed_fixture": False,
        "production_acceptance": False, "certificate_pki_acceptance": False,
        "sha256": {str(path): digest(artifacts / path) for path in relative_files},
        "commands": evidence,
    }, indent=2) + "\n").encode())
    print(json.dumps({"built_only": True, "keychain_operations_run": False,
                      "artifacts": str(artifacts), "production_acceptance": False,
                      "certificate_pki_acceptance": False}))


def run(artifacts: Path, *, store_only: bool = False) -> int:
    private_root(artifacts)
    for name in ("execution-started.json", "execution-result.json"):
        if (artifacts / name).exists() or (artifacts / name).is_symlink():
            raise FixtureError("previous fixture execution is preserved")
    parent = artifacts / "owned-state"
    private_root(parent)
    if any(parent.iterdir()):
        raise FixtureError("existing owned store state is preserved")
    outputs = [artifacts / "store-proof.stdout.json", artifacts / "store-proof.stderr.log"]
    for path in outputs:
        private_file(path, empty=True)
    private_file(artifacts / "build-evidence.json")
    evidence = json.loads((artifacts / "build-evidence.json").read_text())
    if (evidence.get("schema_version") != 1 or evidence.get("executed_fixture") is not False
            or evidence.get("production_acceptance") is not False
            or evidence.get("certificate_pki_acceptance") is not False):
        raise FixtureError("fixture evidence is not a reviewed build-only artifact")
    app = artifacts / "Scoped Profile Crypto Fixture.app"
    binary = app / "Contents/MacOS/scoped-profile-crypto-fixture"
    expected_files = {"reviewed-fixture.mm", "profile_crypto.mm", "profile_crypto.h", str(binary.relative_to(artifacts)),
                      "Scoped Profile Crypto Fixture.app/Contents/Frameworks/libcolossus_mac_profile_crypto.dylib"}
    hashes = evidence.get("sha256")
    if not isinstance(hashes, dict) or set(hashes) != expected_files:
        raise FixtureError("fixture hash manifest is invalid")
    for name, expected in hashes.items():
        path = artifacts / name
        if path != path.resolve(strict=True) or digest(path) != expected:
            raise FixtureError("reviewed fixture artifact changed")
    checked(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)])
    # Mark the attempt before launch; a crash must never make it rerunnable.
    durable_write(artifacts / "execution-started.json", (json.dumps({
        "production_acceptance": False, "certificate_pki_acceptance": False,
        "profile_store_lifecycle_only": store_only,
    }) + "\n").encode())
    with outputs[0].open("r+b") as stdout, outputs[1].open("r+b") as stderr:
        try:
            command = [str(binary), str(parent)]
            if store_only:
                command.append("--store-only")
            process = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=25)
            code = process.returncode
        except subprocess.TimeoutExpired:
            code = 124
    # Do not print paths, snapshots, secrets, or raw native diagnostics.
    durable_write(artifacts / "execution-result.json", (json.dumps({
        "exit_code": code, "production_acceptance": False,
        "certificate_pki_acceptance": False, "chromium_launched": False,
        "profile_store_lifecycle_only": store_only,
        "physical_state_preserved_for_review": True,
    }) + "\n").encode())
    print(json.dumps({"exit_code": code, "production_acceptance": False,
                      "certificate_pki_acceptance": False, "chromium_launched": False}))
    return code


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("build", "run"))
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--native-build", type=Path)
    parser.add_argument("--store-only", action="store_true",
                        help="run initial creation/removal without any Keychain item mutation")
    arguments = parser.parse_args()
    if sys.platform != "darwin":
        raise FixtureError("fixture requires native macOS")
    if not arguments.artifacts.is_absolute():
        raise FixtureError("supply an absolute fresh private artifact path")
    if arguments.operation == "build":
        if arguments.store_only:
            raise FixtureError("store-only selects an explicit run, not a build")
        if arguments.native_build is None:
            raise FixtureError("supply the explicit Debug native build")
        build(arguments.native_build, arguments.artifacts)
        return 0
    if arguments.native_build is not None:
        raise FixtureError("run cannot rebuild or reinterpret the reviewed fixture")
    return run(arguments.artifacts, store_only=arguments.store_only)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (FixtureError, OSError, ValueError, subprocess.SubprocessError):
        print("profile crypto fixture failed; preserve private evidence and owned state", file=sys.stderr)
        raise SystemExit(1)
