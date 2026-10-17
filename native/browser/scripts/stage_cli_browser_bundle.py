#!/usr/bin/env python3
"""Build an offline Linux CLI package from publisher-accepted browser payloads.

Seal with COLOSSUS_CLI_BROWSER_MANIFEST at compile time, or pass --build-cli.
This command never signs a release, promotes component modes, loads an image,
downloads dependencies, or imports personal browser/credential state.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import selectors
import stat
import subprocess
import sys
import tempfile
import time

import component
from pki_fixture_private import private_directory, private_write
from run_probe import stop_process
from stage_macos import rename_exclusive

MAX_CLI_BYTES = 256 * 1024 * 1024
MAX_PAYLOAD_BYTES = 4 * 1024 * 1024 * 1024
MAX_INFO_BYTES = 16 * 1024


class BundleError(Exception):
    """Categorical packaging failure, never native stderr or private material."""


def digest(value: object) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def regular(path: Path, ceiling: int) -> tuple[object, os.stat_result]:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    stream = os.fdopen(descriptor, "rb")
    metadata = os.fstat(stream.fileno())
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 \
            or metadata.st_mode & 0o022 or metadata.st_size > ceiling:
        stream.close()
        raise BundleError("package source is unsafe or oversized")
    return stream, metadata


def public_json(path: Path, ceiling: int) -> dict:
    stream, _ = regular(path, ceiling)
    with stream:
        value = json.loads(stream.read(ceiling + 1))
    if not isinstance(value, dict):
        raise BundleError("publisher payload metadata is invalid")
    return value


def run_cli(cli: Path, *arguments: str) -> dict:
    stream, metadata = regular(cli, MAX_CLI_BYTES)
    stream.close()
    if not metadata.st_mode & 0o111:
        raise BundleError("supply a sealed executable Linux CLI")
    process = subprocess.Popen([str(cli), *arguments], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        env={"PATH": os.defpath, "LANG": "C.UTF-8"}, start_new_session=True)
    output = bytearray()
    deadline = time.monotonic() + 120
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                if time.monotonic() >= deadline:
                    raise BundleError("sealed CLI package verification deadline expired")
                for key, _ in selector.select(timeout=0.1):
                    block = os.read(key.fileobj.fileno(), 4096)
                    if not block:
                        selector.unregister(key.fileobj)
                        break
                    if len(output) + len(block) > MAX_INFO_BYTES:
                        raise BundleError("sealed CLI package verification exceeded output bounds")
                    output.extend(block)
        if process.wait(timeout=max(0.1, deadline - time.monotonic())) != 0:
            raise BundleError("sealed CLI rejected publisher or payload verification")
    finally:
        process.stdout.close()
        stop_process(process)
    value = json.loads(output)
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise BundleError("sealed CLI returned invalid package verification")
    return value


def copy_verified(source: Path, destination: Path, expected: str, size: int, mode: int, ceiling: int) -> None:
    if not digest(expected) or not isinstance(size, int) or isinstance(size, bool) \
            or size < 0 or size > ceiling or mode & ~0o777 or mode & 0o022:
        raise BundleError("package source binding is invalid")
    stream, before = regular(source, ceiling)
    hasher, total, created = hashlib.sha256(), 0, False
    try:
        with stream, destination.open("xb") as output:
            created = True
            os.chmod(destination, mode)
            while block := stream.read(64 * 1024):
                total += len(block)
                if total > size:
                    raise BundleError("package source exceeded its byte binding")
                hasher.update(block)
                output.write(block)
            after = os.fstat(stream.fileno())
            if total != size or before.st_size != size or hasher.hexdigest() != expected \
                    or (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != \
                       (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
                raise BundleError("package source changed or failed its exact digest binding")
    except BaseException:
        if created:
            destination.unlink(missing_ok=True)
        raise


def copy_payload(source: Path, destination: Path, cli: Path, binding: dict) -> None:
    manifest = public_json(source / "manifest.json", 1024 * 1024)
    if not binding.get("publisher_binding_present") or binding.get("publisher_manifest_sha256") != \
            component.digest_file(source / "manifest.json") or binding.get("files") != manifest.get("files"):
        raise BundleError("CLI was not sealed to this accepted publisher manifest")
    native_proof = run_cli(cli, "__browser-bundle-verify", str(source))
    if native_proof.get("publisher_acceptance_verified") is not True \
            or native_proof.get("payload_verified") is not True \
            or native_proof.get("publisher_manifest_sha256") != binding["publisher_manifest_sha256"]:
        raise BundleError("native package verification was not acknowledged")
    release = public_json(source / "browser-release.json", 64 * 1024)
    inventory = component.verify_publisher_accepted_inventory(source / "component", native_proof)
    if inventory.get("platform") != "linux64" or inventory.get("modes") != {"desktop": False, "headless": True}:
        raise BundleError("offline package refuses an unaccepted component mode")
    private_directory(destination / "browser")
    private_directory(destination / "browser/component")
    output = destination / "browser"
    for entry in manifest["files"]:
        copy_verified(source / entry["path"], output / entry["path"], entry["sha256"], entry["size"], 0o600, 8 * 1024 * 1024)
    manifest_size = (source / "manifest.json").stat().st_size
    copy_verified(source / "manifest.json", output / "manifest.json", binding["publisher_manifest_sha256"], manifest_size, 0o600, 1024 * 1024)
    total = 0
    directories = []
    for entry in inventory["files"]:
        target = output / "component" / entry["path"]
        if entry["kind"] == "directory":
            target.mkdir(mode=0o700, parents=False, exist_ok=False)
            directories.append((target, entry["mode"]))
        elif entry["kind"] == "file":
            total += entry["size"]
            if total > MAX_PAYLOAD_BYTES:
                raise BundleError("browser component exceeded the package ceiling")
            copy_verified(source / "component" / entry["path"], target,
                entry["sha256"], entry["size"], entry["mode"], MAX_PAYLOAD_BYTES)
        else:
            raise BundleError("offline package contains an unsupported component entry")
    for directory, mode in reversed(directories):
        directory.chmod(mode)
    image = source / "runtime-image.tar"
    copy_verified(image, output / "runtime-image.tar", release["image_archive_sha256"], image.stat().st_size, 0o600, MAX_PAYLOAD_BYTES)
    copy_verified(cli, destination / "colossus", component.digest_file(cli), cli.stat().st_size, 0o700, MAX_CLI_BYTES)
    result = run_cli(destination / "colossus", "__browser-bundle-verify", str(output))
    if result != native_proof:
        raise BundleError("copied offline payload no longer matches its sealed publisher binding")
    private_write(destination / "OFFLINE-BROWSER.txt", (
        "This package never loads its runtime image during CLI startup.\n"
        "An administrator must explicitly load browser/runtime-image.tar into the local Docker daemon.\n"
        f"Expected archive SHA-256: {release['image_archive_sha256']}\n"
        f"Expected installed immutable image ID: {release['image_id']}\n"
        "The CLI verifies the compiler-bound publisher manifest and every bundled byte before browser composition.\n"
        "It uses only /usr/bin/docker and unix:///var/run/docker.sock; other contexts and runtime selectors are ignored.\n"
        "No personal profiles, keys, or OS trust are imported. Signing and release publication are separate publisher actions.\n"
    ).encode())


def stage(arguments) -> Path:
    if sys.platform != "linux" or os.geteuid() == 0:
        raise BundleError("stage the Linux browser package as a non-root publisher")
    source = component.directory(arguments.payload)
    destination = arguments.destination.absolute()
    parent = destination.parent.resolve(strict=True)
    destination = parent / destination.name
    if destination.exists() or destination.is_symlink() or destination == source or source in destination.parents:
        raise BundleError("offline package destination must be new and outside its payload")
    with tempfile.TemporaryDirectory(prefix=".browser-cli-build-", dir=parent) as build, \
            tempfile.TemporaryDirectory(prefix=".browser-cli-stage-", dir=parent) as stage_root:
        if arguments.build_cli:
            repository = Path(__file__).resolve().parents[3]
            cargo = shutil.which("cargo")
            if not cargo:
                raise BundleError("explicit offline CLI build requires Cargo")
            environment = os.environ.copy()
            environment["COLOSSUS_CLI_BROWSER_MANIFEST"] = str(source / "manifest.json")
            environment["CARGO_TARGET_DIR"] = build
            subprocess.run([cargo, "build", "--offline", "--locked", "--release", "--target",
                "x86_64-unknown-linux-gnu", "-p", "colossus-cli"],
                cwd=repository, env=environment, check=True, timeout=3600)
            cli = Path(build) / "x86_64-unknown-linux-gnu/release/colossus"
        else:
            cli = arguments.cli.resolve(strict=True)
        binding = run_cli(cli, "__browser-bundle-info")
        copy_payload(source, Path(stage_root), cli, binding)
        rename_exclusive(Path(stage_root), destination)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", type=Path, required=True)
    choice = parser.add_mutually_exclusive_group(required=True)
    choice.add_argument("--cli", type=Path)
    choice.add_argument("--build-cli", action="store_true")
    parser.add_argument("--destination", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        print(json.dumps({"offline_package": str(stage(arguments)), "image_loaded": False,
                          "release_signed_or_published": False}))
        return 0
    except (BundleError, component.ComponentError) as error:
        print(str(error), file=sys.stderr)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
        print("offline browser package operation failed", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
