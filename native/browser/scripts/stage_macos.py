#!/usr/bin/env python3
"""Stage and ad-hoc sign an opt-in macOS Chromium development app.

No production packaging or browser download is performed here. The source root
must be an unchanged extraction of the pinned archive produced by component.py.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tempfile

import component


FRAMEWORK = "Chromium Embedded Framework.framework"
HELPER = "Colossus Browser Helper"
HELPERS = (("", ""), (" (Alerts)", ".alerts"), (" (GPU)", ".gpu"),
           (" (Plugin)", ".plugin"), (" (Renderer)", ".renderer"))
IDENTIFIER = "com.obscuritylabs.colossus.chromium-preview"
ENTITLEMENTS = Path(__file__).resolve().parent.parent / "mac/development.entitlements.plist"
MAIN_ENTITLEMENTS = Path(__file__).resolve().parent.parent / "mac/development-main.entitlements.plist"
DESKTOP_INFO_PLIST = Path(__file__).resolve().parents[3] / "apps/desktop/src-tauri/Info.plist"
DICTATION_RESOURCES = ("ggml-tiny.en.bin", "models.json", "LICENSE-MIT")
PROFILE_CRYPTO_LIBRARY = "libcolossus_mac_profile_crypto.dylib"
PROFILE_CRYPTO_BUILD = "profile-crypto-development-build.json"
PROFILE_CRYPTO_CLAIM = "profile-crypto-development.json"
NETWORK_ENVELOPE_RESOURCE = "colossus-network-envelope.policy"
NETWORK_ENVELOPE_MARKER = "COLOSSUS_MAC_NETWORK_ENVELOPE_V1"
NETWORK_ENVELOPE_BUILD = "network-envelope-development-build.json"
NETWORK_ENVELOPE_METADATA = {
    "schema_version": 1, "development_only": True, "build_type": "Debug",
    "production_accepted": False, "native_acceptance": False,
}
PROFILE_CRYPTO_METADATA = {
    "schema_version": 1, "development_only": True, "build_type": "Debug",
    "production_accepted": False, "api": "unsupported-dyld-security-spi",
}


def profile_crypto_dependency(executable: Path) -> None:
    """The adapter must be an ordinary launch dependency, never loader injection."""
    result = subprocess.run(["/usr/bin/otool", "-L", str(executable)],
                            check=True, text=True, capture_output=True)
    dependencies = [line.strip().split(" ", 1)[0] for line in result.stdout.splitlines()[1:]]
    if "@rpath/" + PROFILE_CRYPTO_LIBRARY not in dependencies:
        raise component.ComponentError("development profile crypto requires the signed launch dependency")


def network_envelope_binding(source: Path, app: Path, identifier: str) -> bytes:
    """Bind a finite native-only policy resource to this exact fresh app path."""
    source = source.absolute()
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(descriptor, "rb") as input_file:
        before = os.fstat(input_file.fileno())
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.geteuid() or
                before.st_nlink != 1 or before.st_mode & 0o077 or
                before.st_size == 0 or before.st_size > 32 * 1024):
            raise component.ComponentError("network envelope source is not private and bounded")
        data = input_file.read(32 * 1024 + 1)
        after = os.fstat(input_file.fileno())
        if (len(data) != before.st_size or len(data) > 32 * 1024 or
                (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) !=
                (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)):
            raise component.ComponentError("network envelope source changed during binding")
    try:
        fields = data.decode("ascii").split("\n")
    except UnicodeDecodeError as error:
        raise component.ComponentError("network envelope has invalid encoding") from error
    if (len(fields) != 10 or fields[-1] or fields[0] != NETWORK_ENVELOPE_MARKER or
            not fields[1].isdecimal() or int(fields[1]) in (0, 2**32 - 1) or
            int(fields[1]) >= 2**32 or not fields[2].isdecimal() or
            not 1 <= int(fields[2]) <= 65535):
        raise component.ComponentError("network envelope has invalid fixed fields")
    allocation, bundle, profile, broker, personal = map(
        Path, (fields[3], fields[4], fields[6], fields[7], fields[8]))
    if (bundle != app or allocation != app.parent or profile != allocation / "profile" or
            fields[5] != identifier or source.parent != allocation or
            not all(path.is_absolute() and path.resolve() == path for path in
                    (allocation, profile, broker, personal)) or
            not profile.is_relative_to(allocation) or broker.is_relative_to(allocation)):
        raise component.ComponentError("network envelope does not bind this owned allocation")
    if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", fields[5]):
        raise component.ComponentError("network envelope has invalid bundle identity")
    return data


def verify_source(root: Path, platform: str) -> Path:
    root = component.directory(root)
    # Sources provisioned into a cache are always cache/sources/<locked name>.
    # Re-check against archive bytes, never treating the receipt as authority.
    pin = component.load_lock()["archives"][platform]
    if root.parent.name != "sources" or root.name != pin["filename"].removesuffix(".tar.bz2"):
        raise component.ComponentError("CEF root does not match the pinned cache extraction")
    archive = root.parent.parent / f"cef-{platform}.tar.bz2"
    component.verify_archive(archive, pin)
    receipt = component.load_json(root / component.SOURCE_MANIFEST)
    expected = component.source_archive_inventory(archive, root.name)
    if (receipt.get("schema_version") != 1 or receipt.get("archive_sha256") != pin["sha256"]
            or not component.source_inventory_matches(expected, receipt.get("files"))
            or not component.source_inventory_matches(
                expected, component.inventory_files(root, component.SOURCE_MANIFEST))):
        raise component.ComponentError("CEF source bytes differ from the pinned archive")
    return root


def regular_executable(path: Path) -> Path:
    component.digest_file(path)
    if not path.stat().st_mode & 0o111:
        raise component.ComponentError(f"missing executable permission: {path}")
    return path


def run(command: list[str]) -> None:
    subprocess.run(command, check=True)


def rename_exclusive(source: Path, destination: Path) -> None:
    """Publish one object atomically without replacing any destination entry."""
    library = ctypes.CDLL(None, use_errno=True)
    try:
        if sys.platform == "darwin":
            rename = library.renamex_np
            rename.argtypes = (ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint)
            arguments = (os.fsencode(source), os.fsencode(destination), 0x00000004)  # RENAME_EXCL
        elif sys.platform.startswith("linux"):
            # The POSIX fixture suite uses Linux's equivalent no-replace API.
            rename = library.renameat2
            rename.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_int,
                               ctypes.c_char_p, ctypes.c_uint)
            arguments = (-100, os.fsencode(source), -100, os.fsencode(destination), 1)
        else:
            raise component.ComponentError("exclusive app publication is unavailable on this host")
    except AttributeError as error:
        raise component.ComponentError("exclusive app publication is unavailable on this host") from error
    rename.restype = ctypes.c_int
    if rename(*arguments) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), os.fspath(destination))


def object_identity(path: Path) -> tuple[int, int, int]:
    info = path.lstat()
    return info.st_dev, info.st_ino, stat.S_IFMT(info.st_mode)


def rollback_published(path: Path, identity: tuple[int, int, int]) -> None:
    try:
        if object_identity(path) != identity:
            return
    except FileNotFoundError:
        return
    # Reclaim into an owner-private directory before recursive cleanup. A
    # replacement at the public pathname must never become the deletion target.
    quarantine = Path(tempfile.mkdtemp(prefix=".cef-app-rollback-", dir=path.parent))
    reclaimed = quarantine / path.name
    try:
        rename_exclusive(path, reclaimed)
    except OSError:
        quarantine.rmdir()
        raise
    if object_identity(reclaimed) != identity:
        try:
            rename_exclusive(reclaimed, path)
        except OSError as error:
            # This directory is deliberately outside TemporaryDirectory cleanup.
            raise component.ComponentError(
                f"a raced unrelated staging destination was preserved at {reclaimed}"
            ) from error
        quarantine.rmdir()
        return
    shutil.rmtree(quarantine)


def publish_artifacts(artifacts: list[tuple[Path, Path]]) -> None:
    published = []
    try:
        for source, destination in artifacts:
            identity = object_identity(source)
            rename_exclusive(source, destination)
            published.append((destination, identity))
    except BaseException:
        for destination, identity in reversed(published):
            rollback_published(destination, identity)
        raise


def sign_app(app: Path, helpers: list[Path], platform: str, *, host: bool = False,
             profile_crypto_development: bool = False,
             network_envelope_development: bool = False) -> None:
    arch = "arm64" if platform == "macosarm64" else "x86_64"
    framework = app / "Contents/Frameworks" / FRAMEWORK
    # A copied pinned framework includes libcef_sandbox.dylib: helpers load it
    # before loading Chromium. Sign every nested dylib before its container.
    libraries = sorted((framework / "Versions/A/Libraries").glob("*.dylib"))
    if profile_crypto_development:
        libraries.append(app / "Contents/Frameworks" / PROFILE_CRYPTO_LIBRARY)
    with (app / "Contents/Info.plist").open("rb") as source:
        main_executable = plistlib.load(source)["CFBundleExecutable"]
    bodies = ([app / "Contents/MacOS" / (main_executable + " Body")]
              + [helper / "Contents/MacOS" / (helper.stem + " Body") for helper in helpers]
              if network_envelope_development else [])
    for binary in [app / "Contents/MacOS" / main_executable,
                   *(helper / "Contents/MacOS" / helper.stem for helper in helpers),
                   framework / "Versions/A/Chromium Embedded Framework", *libraries,
                   *bodies]:
        run(["/usr/bin/lipo", str(binary), "-verify_arch", arch])
    for library in libraries:
        run(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", str(library)])
    run(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", str(framework)])
    for body in bodies:
        run(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none",
             "--options", "runtime", "--entitlements", str(ENTITLEMENTS), str(body)])
        run(["/usr/bin/codesign", "--verify", "--strict", str(body)])
    for target in [*helpers, app]:
        command = ["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none",
                   "--options", "runtime"]
        if not network_envelope_development:
            entitlements = MAIN_ENTITLEMENTS if target == app and not host else ENTITLEMENTS
            command.extend(["--entitlements", str(entitlements)])
        run([*command, str(target)])
    # Verify recursively, but never use --deep signing to choose entitlements.
    run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)])


def stage_app(cef_root: Path, native_build: Path, executable: Path, app: Path,
              platform: str, identifier: str = IDENTIFIER,
              dictation_resources: Path | None = None, *, host: bool = False,
              profile_crypto_development: bool = False,
              network_envelope_policy: Path | None = None) -> Path:
    if platform not in ("macosarm64", "macosx64"):
        raise component.ComponentError("development app staging requires a macOS platform")
    if not re.fullmatch(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+", identifier):
        raise component.ComponentError("invalid development app identifier")
    cef_root = verify_source(cef_root, platform)
    native_build = component.directory(native_build)
    executable = regular_executable(executable.absolute())
    if host and (executable.name != "colossus-native-browser-host" or dictation_resources is not None):
        raise component.ComponentError("native host staging requires the fixed host executable and no Desktop assets")
    if profile_crypto_development and not host:
        raise component.ComponentError("unsupported profile crypto is restricted to the explicit native development host")
    network_envelope_development = network_envelope_policy is not None
    if network_envelope_development and (not host or not profile_crypto_development):
        raise component.ComponentError("the network envelope requires the explicit private Debug host and owned profile adapter")
    crypto_build = native_build / PROFILE_CRYPTO_BUILD
    if host and crypto_build.exists() and not profile_crypto_development:
        raise component.ComponentError("a profile crypto Debug build requires explicit development staging")
    if profile_crypto_development:
        if component.load_json(crypto_build) != PROFILE_CRYPTO_METADATA:
            raise component.ComponentError("unsupported or non-Debug profile crypto build")
        component.digest_file(native_build / PROFILE_CRYPTO_LIBRARY)
        profile_crypto_dependency(executable)
    if dictation_resources is not None:
        dictation_resources = component.directory(dictation_resources)
        for name in DICTATION_RESOURCES:
            component.digest_file(dictation_resources / name)
    executable_name = component.safe_path(executable.name)
    app = app.absolute()
    if app.suffix != ".app" or app.exists() or app.is_symlink():
        raise component.ComponentError("select a new .app destination; existing apps are never replaced")
    manifest = app.with_name(app.name + ".browser-component.json")
    if manifest.exists() or manifest.is_symlink():
        raise component.ComponentError("an inventory already exists for this development app")
    policy_bytes = (network_envelope_binding(network_envelope_policy, app, identifier)
                    if network_envelope_policy is not None else None)
    front = native_build / "envelope/colossus-native-browser-host"
    if network_envelope_development:
        if component.load_json(native_build / NETWORK_ENVELOPE_BUILD) != NETWORK_ENVELOPE_METADATA:
            raise component.ComponentError("network envelope requires the explicit Debug front build")
        regular_executable(front)
    parent = component.directory(app.parent, create=True)
    helper_root = component.directory(native_build / "helpers")
    component.inventory_files(helper_root, component.MANIFEST)
    for suffix, _ in HELPERS:
        name = HELPER + suffix
        helper = helper_root / (name + ".app")
        regular_executable(helper / "Contents/MacOS" / name)
        if network_envelope_development:
            regular_executable(helper / "Contents/MacOS" / (name + " Body"))
        with (helper / "Contents/Info.plist").open("rb") as source:
            info = plistlib.load(source)
        if info.get("CFBundleExecutable") != name or info.get("CFBundlePackageType") != "APPL":
            raise component.ComponentError(f"invalid helper bundle identity: {name}")
    with tempfile.TemporaryDirectory(prefix=".cef-app-stage-", dir=parent) as temporary:
        staged = Path(temporary) / app.name
        contents = staged / "Contents"
        macos = contents / "MacOS"
        resources = contents / "Resources"
        frameworks = contents / "Frameworks"
        for directory in (macos, resources, frameworks):
            directory.mkdir(parents=True, exist_ok=True)
        if dictation_resources is not None:
            dictation = resources / "dictation"
            dictation.mkdir()
            for name in DICTATION_RESOURCES:
                shutil.copy2(dictation_resources / name, dictation / name)
        if network_envelope_development:
            shutil.copy2(front, macos / executable_name)
            shutil.copy2(executable, macos / (executable_name + " Body"))
            policy = resources / NETWORK_ENVELOPE_RESOURCE
            with policy.open("xb") as output:
                output.write(policy_bytes)
                output.flush()
                os.fsync(output.fileno())
            policy.chmod(0o400)
        else:
            shutil.copy2(executable, macos / executable_name)
        framework = frameworks / FRAMEWORK
        version = framework / "Versions/A"
        shutil.copytree(cef_root / "Release" / FRAMEWORK, version, symlinks=True)
        for name in ("Chromium Embedded Framework", "Libraries", "Resources"):
            (framework / name).symlink_to("Versions/A/" + name)
        (framework / "Versions/Current").symlink_to("A", target_is_directory=True)
        for resource in ("icudtl.dat", "resources.pak", "chrome_100_percent.pak",
                         "chrome_200_percent.pak"):
            if not (version / "Resources" / resource).is_file():
                raise component.ComponentError(f"CEF framework resource is absent: {resource}")
        if not (version / "Libraries/libcef_sandbox.dylib").is_file():
            raise component.ComponentError("CEF sandbox library is absent")
        helpers = []
        for suffix, identifier_suffix in HELPERS:
            name = HELPER + suffix
            helper = frameworks / (name + ".app")
            shutil.copytree(helper_root / helper.name, helper, symlinks=True)
            plist = helper / "Contents/Info.plist"
            with plist.open("rb") as source:
                info = plistlib.load(source)
            info["CFBundleIdentifier"] = identifier + ".helper" + identifier_suffix
            with plist.open("wb") as output:
                plistlib.dump(info, output)
            helpers.append(helper)
        if host:
            main_info = {"LSUIElement": True}
        else:
            with DESKTOP_INFO_PLIST.open("rb") as source:
                main_info = plistlib.load(source)
        # Preserve Desktop's microphone privacy description for its default
        # offline dictation feature. Helpers receive neither description nor
        # the main process's microphone entitlement.
        main_info.update({
                "CFBundleExecutable": executable_name,
                "CFBundleIdentifier": identifier,
                "CFBundleName": "Colossus Browser Host" if host else "Colossus Chromium Preview",
                "CFBundlePackageType": "APPL",
                "CFBundleInfoDictionaryVersion": "6.0",
                "CFBundleVersion": "0.1.0", "CFBundleShortVersionString": "0.1.0",
                "LSMinimumSystemVersion": "13.0", "NSHighResolutionCapable": True,
                "NSSupportsAutomaticGraphicsSwitching": True,
                "LSEnvironment": {"MallocNanoZone": "0"},
            })
        with (contents / "Info.plist").open("wb") as output:
            plistlib.dump(main_info, output)
        notices = resources / "cef-notices"
        notices.mkdir()
        for name in ("LICENSE.txt", "CREDITS.html"):
            shutil.copy2(cef_root / name, notices / name)
        if profile_crypto_development:
            shutil.copy2(native_build / PROFILE_CRYPTO_LIBRARY, frameworks / PROFILE_CRYPTO_LIBRARY)
            with (resources / PROFILE_CRYPTO_CLAIM).open("x", encoding="utf-8") as output:
                json.dump({**PROFILE_CRYPTO_METADATA, "temporary_profiles_only": True,
                           "certificate_pki_accepted": False}, output, sort_keys=True)
                output.write("\n")
        component.inventory_files(staged, component.MANIFEST)
        sign_app(staged, helpers, platform, host=host,
                 profile_crypto_development=profile_crypto_development,
                 network_envelope_development=network_envelope_development)
        # This developer consistency receipt lives beside the signed app.
        # It does not establish publisher identity or promote release modes.
        temporary_manifest = Path(temporary) / component.MANIFEST
        component.installed_inventory(staged, platform, "Contents/MacOS/" + executable_name,
                                       temporary_manifest)
        component.verify_installed(staged, temporary_manifest)
        run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(staged)])
        publish_artifacts([(staged, app), (temporary_manifest, manifest)])
    return app / "Contents/MacOS" / executable_name


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cef-root", type=Path, required=True)
    parser.add_argument("--native-build", type=Path, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--platform", choices=("macosarm64", "macosx64"), required=True)
    parser.add_argument("--identifier", default=IDENTIFIER)
    parser.add_argument("--dictation-resources", type=Path,
                        help="prepared Desktop dictation assets (three fixed files)")
    options = parser.parse_args()
    try:
        if sys.platform != "darwin":
            raise component.ComponentError("macOS development staging requires codesign and lipo")
        print(stage_app(options.cef_root, options.native_build, options.executable,
                        options.app, options.platform, options.identifier, options.dictation_resources))
        return 0
    except (component.ComponentError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Chromium development staging: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
