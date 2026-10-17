#!/usr/bin/env python3
"""Validate the explicit patched-CEF development lane without loading CEF.

These receipts are developer consistency evidence, never publisher admission or
production authority. Stock archives are rejected on the owned-process path.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

import component

ROOT = Path(__file__).resolve().parents[3]
PATCH = ROOT / "native/browser/component/patches/macos-owned-process"
SOURCE_RECEIPT = ".cef-owned-process-source.json"
BUILD_RECEIPT = "owned-process-development-build.json"
NATIVE_BUILD_RECEIPT = "owned-process-native-artifacts.json"
CLAIM = "owned-process-development.json"
PRIVATE_EXPORTS = ("colossus_owned_process_install", "colossus_owned_helper_install")
BODY_EXPORT = "colossus_owned_body_main"
MAIN_INSTALL_EXPORT = "colossus_cef_owned_process_install"
FRONT_EXPORTS = ("colossus_verified_owned_image", "colossus_final_owned_role_policy",
                 "colossus_owned_library_verify")
HELPER_NAMES = tuple("Colossus Browser Helper" + suffix
                    for suffix in ("", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"))
NATIVE_ARTIFACTS = (("libcolossus_cef.a", "archive"),
                    ("envelope/colossus-native-browser-host", "front")) + tuple(
    (f"helpers/{name}.app/Contents/MacOS/{name}{suffix}", kind)
    for name in HELPER_NAMES for suffix, kind in (("", "front"), (" Body.dylib", "body")))
NATIVE_SOURCES = (
    "native/browser/CMakeLists.txt", "native/browser/mac/owned_process.cmake",
    "native/browser/mac/network_envelope.cmake", "native/browser/mac/helper-Info.plist.in",
    "native/browser/mac/network_envelope.cc", "native/browser/mac/network_envelope.h",
    "native/browser/mac/sandbox/policies.lock.json",
    "native/browser/include/colossus_cef.h", "native/browser/include/colossus_cef_presentation.h",
    "native/browser/include/colossus_cef_transfer.h", "native/browser/src/host_internal.h",
    "native/browser/src/host.cc", "native/browser/src/client.cc", "native/browser/src/broker_tls.h",
    "native/browser/src/bootstrap_network.h",
    "native/browser/src/control.cc", "native/browser/src/platform.cc",
    "native/browser/src/platform_environment.h", "native/browser/src/platform_mac.mm",
    "native/browser/src/mac_helper_programs.cc", "native/browser/src/mac_helper_programs.h",
    "native/browser/src/presentation.cc", "native/browser/src/transfer.cc",
    "native/browser/src/profile.cc", "native/browser/src/synthetic_profile.cc",
    "native/browser/src/synthetic_profile.h",
    "native/browser/process-broker/front_loader.c", "native/browser/process-broker/main_entry.cc",
    "native/browser/process-broker/helper_body.mm", "native/browser/process-broker/fixed_policy.c",
    "native/browser/process-broker/policy_transition.c", "native/browser/process-broker/policy_transition.h",
    "native/browser/process-broker/custody_policy.c", "native/browser/process-broker/custody_policy.h",
    "native/browser/process-broker/custody_probe.c", "native/browser/process-broker/custody_probe.h",
    "native/browser/process-broker/image_verify.c", "native/browser/process-broker/image_verify.h",
    "native/browser/scripts/mac_network_policies.py", "native/browser/scripts/owned_process_component.py",
)


def symbols(image: Path, required: tuple[str, ...]) -> None:
    result = subprocess.run(["/usr/bin/nm", "-gjU", str(image)], check=True,
                            capture_output=True, text=True, timeout=30)
    if len(result.stdout) > 16 * 1024 * 1024:
        raise component.ComponentError("native export table exceeds its bound")
    exported = set(result.stdout.splitlines())
    if any("_" + name not in exported for name in required):
        raise component.ComponentError("owned-process lane requires actual defined private native exports")


def macho_type(image: Path, expected: str) -> None:
    component.digest_file(image)
    # Apple otool treats a pathname ending in "(Renderer)" as archive-member
    # notation even when passed as one argv element. Inspect the opened regular
    # image through an inherited descriptor instead of rewriting its filename.
    with image.open("rb") as source:
        descriptor = source.fileno()
        result = subprocess.run(["/usr/bin/otool", "-hv", f"/dev/fd/{descriptor}"],
                                pass_fds=(descriptor,), check=True,
                                capture_output=True, text=True, timeout=30)
    headers = re.findall(r"^\s*MH_(?:MAGIC|CIGAM)(?:_64)?\s+[^\n]+$", result.stdout, re.MULTILINE)
    if len(result.stdout) > 16 * 1024 or not headers or any(
            not re.search(r"\b" + expected + r"\b", header) for header in headers):
        raise component.ComponentError("owned native image must contain only Mach-O " + expected + " slices")


def dylib(image: Path) -> None:
    macho_type(image, "DYLIB")
    symbols(image, (BODY_EXPORT,))


def framework(root: Path) -> Path:
    return root / "Release/Chromium Embedded Framework.framework/Chromium Embedded Framework"


def metadata(root: Path, platform: str) -> dict:
    if platform not in ("macosarm64", "macosx64"):
        raise component.ComponentError("owned process source requires a macOS platform")
    manifest = component.load_json(PATCH / "manifest.json")
    if component.digest_file(PATCH / "chromium.patch") != manifest["patch_sha256"]:
        raise component.ComponentError("owned Chromium patch differs from its reviewed lock")
    for name, digest in manifest["overlay_sha256"].items():
        if component.digest_file(ROOT / "native/browser/process-broker" / name) != digest:
            raise component.ComponentError("owned process overlay differs from its reviewed lock")
    header = (root / "include/cef_version.h").read_text()
    if '#define CEF_VERSION "' + component.CEF_VERSION + '"' not in header:
        raise component.ComponentError("owned process CEF version differs from pin")
    image = framework(root)
    symbols(image, PRIVATE_EXPORTS)
    return {"schema_version": 1, "development_only": True, "production_accepted": False,
            "native_acceptance": False, "platform": platform,
            "cef_commit": manifest["cef_commit"], "chromium_revision": manifest["chromium_revision"],
            "patch_sha256": manifest["patch_sha256"], "overlay_sha256": manifest["overlay_sha256"],
            "cef_lazy_sampler_seed_patch_sha256": manifest["cef_lazy_sampler_seed_patch_sha256"],
            "private_exports": list(PRIVATE_EXPORTS), "framework_sha256": component.digest_file(image)}


def record_source(root: Path, platform: str, chromium: Path) -> Path:
    """Record actual reviewed source plus built exports; performs no source fetch."""
    root = component.directory(root)
    chromium = component.directory(chromium)
    manifest = component.load_json(PATCH / "manifest.json")
    cef = chromium / "cef"
    if subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=cef, text=True).strip() != manifest["cef_commit"]:
        raise component.ComponentError("CEF checkout differs from exact reviewed commit")
    lazy_patch = "patch/patches/base_rand_util_lazy_seed.patch"
    lazy_bytes = subprocess.check_output(["git", "show", "HEAD:" + lazy_patch], cwd=cef)
    if (hashlib.sha256(lazy_bytes).hexdigest() != manifest["cef_lazy_sampler_seed_patch_sha256"] or
            (cef / lazy_patch).read_bytes() != lazy_bytes):
        raise component.ComponentError("CEF lazy sampler patch differs from the pinned compatibility provenance")
    original = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=chromium, text=True).strip()
    tagged = subprocess.check_output(["git", "rev-parse", manifest["chromium_revision"]], cwd=chromium, text=True).strip()
    if original != tagged:
        raise component.ComponentError("Chromium checkout differs from exact compatibility revision")
    # Derive the exact patched bytes from independently checked upstream bytes.
    # Reverse applicability alone would also accept unrelated edits in a file.
    with tempfile.TemporaryDirectory(prefix="colossus-owned-source-check-") as temporary:
        expected_root = Path(temporary)
        for name, expected in manifest["source_sha256"].items():
            before = subprocess.check_output(["git", "show", "HEAD:" + name], cwd=chromium)
            if hashlib.sha256(before).hexdigest() != expected:
                raise component.ComponentError("Chromium baseline differs from locked patch source")
            target = expected_root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(before)
        subprocess.run(["git", "apply", str(PATCH / "chromium.patch")], cwd=expected_root, check=True)
        if any((expected_root / name).read_bytes() != (chromium / name).read_bytes() for name in manifest["source_sha256"]):
            raise component.ComponentError("Chromium launch source differs from the exact reviewed patch")
    for name, destination in manifest["overlay_sources"].items():
        expected = (ROOT / "native/browser/process-broker" / name).read_bytes()
        if name == "client.cc":
            expected = expected.replace(b'#include "client.h"', b'#include "base/process/colossus_owned_process_client.h"', 1)
        if (chromium / destination).read_bytes() != expected:
            raise component.ComponentError("built-source client overlay differs from locked bytes")
    value = metadata(root, platform)
    value["files"] = component.inventory_files(root, SOURCE_RECEIPT)
    return component.write_manifest(root, SOURCE_RECEIPT, value)


def verify_source(root: Path, platform: str) -> Path:
    root = component.directory(root)
    value = component.load_json(root / SOURCE_RECEIPT)
    expected = metadata(root, platform)
    expected["files"] = component.inventory_files(root, SOURCE_RECEIPT)
    if value != expected:
        raise component.ComponentError("patched CEF source receipt or full payload changed")
    return root


def build_metadata(root: Path, platform: str) -> dict:
    value = metadata(root, platform)
    return {**value, "build_type": "Debug", "main_body": "dylib", "creator_descriptor": 7,
            "cef_source_receipt_sha256": component.digest_file(root / SOURCE_RECEIPT),
            "native_source_sha256": native_source_hashes()}


def native_source_hashes() -> dict:
    sources = [ROOT / name for name in NATIVE_SOURCES]
    sources.extend(sorted((ROOT / "native/browser/mac/sandbox").glob("*.sb")))
    return {path.relative_to(ROOT).as_posix(): component.digest_file(path) for path in sources}


def write_build_configuration(root: Path, native: Path, platform: str) -> Path:
    """Configure evidence alone never satisfies the completed-build gate."""
    native = component.directory(native, create=True)
    expected = build_metadata(root, platform)
    path = native / BUILD_RECEIPT
    if path.exists() or path.is_symlink():
        if component.load_json(path) != expected:
            raise component.ComponentError("owned-process configuration is stale; choose a fresh build directory")
        return path
    return component.write_manifest(native, BUILD_RECEIPT, expected)


def verify_build_configuration(root: Path, native: Path, platform: str) -> dict:
    verify_source(root, platform)
    value = component.load_json(native / BUILD_RECEIPT)
    if value != build_metadata(root, platform):
        raise component.ComponentError("owned-process configuration does not bind this patched component and native source")
    return value


def native_artifact_inventory(native: Path) -> list[dict]:
    """Inspect fixed CMake outputs without loading any front, body or CEF."""
    records = []
    for relative, kind in NATIVE_ARTIFACTS:
        image = native / relative
        before = component.digest_file(image)
        if kind == "archive":
            symbols(image, (MAIN_INSTALL_EXPORT,))
        elif kind == "front":
            if not image.stat().st_mode & 0o111:
                raise component.ComponentError("owned frontend must have executable permissions")
            macho_type(image, "EXECUTE")
            symbols(image, FRONT_EXPORTS)
        else:
            dylib(image)
        if component.digest_file(image) != before:
            raise component.ComponentError("owned native artifact changed during inspection")
        info = image.stat()
        records.append({"path": relative, "kind": kind, "sha256": before,
                        "size": info.st_size, "mode": info.st_mode & 0o7777})
    return records


def native_build_metadata(native: Path, platform: str) -> dict:
    return {"schema_version": 1, "development_only": True, "production_accepted": False,
            "native_acceptance": False, "platform": platform,
            "build_receipt_sha256": component.digest_file(native / BUILD_RECEIPT),
            "files": native_artifact_inventory(native)}


def write_native_build(root: Path, native: Path, platform: str) -> Path:
    """Called by CMake only after the archive and every fixed front/body target."""
    native = component.directory(native)
    verify_build_configuration(root, native, platform)
    expected = native_build_metadata(native, platform)
    path = native / NATIVE_BUILD_RECEIPT
    if path.exists() or path.is_symlink():
        component.digest_file(path)
        if component.load_json(path) == expected:
            return path
    # Rebuilt output receives a new receipt only after all validation succeeds.
    # Atomic replacement leaves either the old complete receipt or the new one.
    with tempfile.TemporaryDirectory(prefix=".owned-process-artifacts-", dir=native) as temporary:
        fresh = component.write_manifest(Path(temporary), NATIVE_BUILD_RECEIPT, expected)
        os.replace(fresh, path)
    return path


def verify_build(root: Path, native: Path, platform: str) -> dict:
    native = component.directory(native)
    value = verify_build_configuration(root, native, platform)
    completed = component.load_json(native / NATIVE_BUILD_RECEIPT)
    if completed != native_build_metadata(native, platform):
        raise component.ComponentError("owned-process completed native artifact receipt changed")
    return {**value, "native_artifact_receipt_sha256": component.digest_file(native / NATIVE_BUILD_RECEIPT)}


def installed_inventory(app: Path, platform: str, executable: str, manifest: Path) -> Path:
    # This distinct receipt never calls the stock archive inventory function or
    # asserts that custom compiled CEF bytes came from the stock archive SHA.
    records = component.inventory_files(app, None)
    claim = component.load_json(app / "Contents/Resources" / CLAIM)
    document = {"schema_version": 1, "protocol_version": 1, "component": "colossus-browser",
                "source_kind": "patched-chromium-development", "platform": platform,
                "target": component.load_lock()["archives"][platform]["target"],
                "cef_version": component.CEF_VERSION, "chromium_version": component.CHROMIUM_VERSION,
                "executable": executable, "modes": {"desktop": False, "headless": False},
                "owned_process": claim, "files": records}
    return component.write_manifest(manifest.parent, manifest.name, document)


def verify_installed(app: Path, manifest: Path) -> dict:
    value = component.load_json(manifest)
    fields = {"schema_version", "protocol_version", "component", "source_kind", "platform", "target", "cef_version", "chromium_version", "executable", "modes", "owned_process", "files"}
    if set(value) != fields or value.get("schema_version") != 1 or value.get("protocol_version") != 1 or value.get("component") != "colossus-browser" or value.get("platform") not in ("macosarm64", "macosx64") or value.get("cef_version") != component.CEF_VERSION or value.get("chromium_version") != component.CHROMIUM_VERSION or value.get("executable") != "Contents/MacOS/colossus-native-browser-host" or value.get("source_kind") != "patched-chromium-development" or value.get("modes") != {"desktop": False, "headless": False} or value.get("files") != component.inventory_files(app, None):
        raise component.ComponentError("owned-process installed inventory changed or claims stock/production authority")
    if value.get("target") != component.load_lock()["archives"][value["platform"]]["target"]:
        raise component.ComponentError("owned-process installed target differs from platform")
    claim = value.get("owned_process")
    if not isinstance(claim, dict) or claim.get("development_only") is not True or claim.get("production_accepted") is not False or claim.get("native_acceptance") is not False or claim.get("runtime_available") is not False or claim.get("complete_helper_cleanup_accepted") is not False or claim != component.load_json(app / "Contents/Resources" / CLAIM):
        raise component.ComponentError("owned-process sealed app claim changed")
    image = app / "Contents/Frameworks/Chromium Embedded Framework.framework/Versions/A/Chromium Embedded Framework"
    symbols(image, PRIVATE_EXPORTS)
    bodies = [app / "Contents/MacOS/colossus-native-browser-host Body.dylib"]
    bodies.extend((app / "Contents/Frameworks").glob("Colossus Browser Helper*.app/Contents/MacOS/* Body.dylib"))
    if len(bodies) != 6:
        raise component.ComponentError("owned-process app lacks exact Main and five helper bodies")
    for body in bodies: dylib(body)
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cef-root", type=Path, required=True)
    parser.add_argument("--platform", choices=("macosarm64", "macosx64"), required=True)
    parser.add_argument("--record-source", type=Path, metavar="CHROMIUM_SRC")
    parser.add_argument("--write-build", type=Path, help="record configure-only source and framework metadata")
    parser.add_argument("--write-native-build", type=Path,
                        help="record completed fixed CMake archive, fronts and helper dylibs without loading them")
    parser.add_argument("--verify-build", type=Path)
    args = parser.parse_args()
    try:
        root = args.cef_root.resolve(strict=True)
        if args.record_source:
            print(record_source(root, args.platform, args.record_source))
        else:
            verify_source(root, args.platform)
        if args.write_build:
            write_build_configuration(root, args.write_build, args.platform)
        if args.write_native_build:
            write_native_build(root, args.write_native_build, args.platform)
        if args.verify_build:
            verify_build(root, args.verify_build, args.platform)
        return 0
    except (component.ComponentError, OSError, subprocess.SubprocessError, ValueError) as error:
        print("Owned process component: " + str(error))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
