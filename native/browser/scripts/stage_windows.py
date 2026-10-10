#!/usr/bin/env python3
"""Stage the pinned Windows CEF bootstrap and debug Desktop client DLL.

The result is a developer component with both accepted modes false. Publisher
signing, installed acceptance and runtime authority containment remain separate.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

import component
import stage_macos

PROGRAM = "colossus-chromium-preview"
HOST_PROGRAM = "colossus-native-browser-host"
MAX_PE_BYTES = 512 * 1024**2
RESOURCES = ("chrome_100_percent.pak", "chrome_200_percent.pak", "resources.pak", "icudtl.dat")


def inspect_client(path: Path) -> None:
    """Require an x64 DLL with the real bootstrap export and delayed CEF import."""
    component.digest_file(path)
    if path.stat().st_size > MAX_PE_BYTES:
        raise component.ComponentError("Windows client DLL exceeds its bound")
    data = path.read_bytes()

    def read(offset: int, length: int) -> bytes:
        if offset < 0 or offset + length > len(data):
            raise component.ComponentError("truncated Windows client DLL")
        return data[offset:offset + length]

    def integer(offset: int) -> int:
        return struct.unpack("<I", read(offset, 4))[0]

    if read(0, 2) != b"MZ":
        raise component.ComponentError("Windows client is not a PE image")
    pe = integer(60)
    if read(pe, 4) != b"PE\0\0":
        raise component.ComponentError("Windows client PE signature is invalid")
    machine, sections = struct.unpack("<HH", read(pe + 4, 4))
    optional_size, flags = struct.unpack("<HH", read(pe + 20, 4))
    optional = pe + 24
    if machine != 0x8664 or not flags & 0x2000 or read(optional, 2) != b"\x0b\x02":
        raise component.ComponentError("Windows preview requires an x64 PE32+ client DLL")
    if not 1 <= sections <= 96 or optional_size < 224:
        raise component.ComponentError("Windows client PE headers are unsupported")
    read(optional, optional_size)
    directory_count = integer(optional + 108)
    if not 14 <= directory_count <= (optional_size - 112) // 8:
        raise component.ComponentError("Windows client PE directory count is unsupported")
    headers = []
    for index in range(sections):
        header = optional + optional_size + 40 * index
        virtual_size, rva, raw_size, raw_offset = struct.unpack("<IIII", read(header + 8, 16))
        headers.append((rva, max(virtual_size, raw_size), raw_offset, raw_size))

    def offset(rva: int, length: int = 1) -> int:
        for start, extent, raw, raw_size in headers:
            relative = rva - start
            if 0 <= relative < extent and relative + length <= raw_size:
                read(raw + relative, length)
                return raw + relative
        raise component.ComponentError("Windows client has an invalid PE directory address")

    def name(rva: int) -> bytes:
        start = offset(rva)
        end = data.find(b"\0", start, min(start + 257, len(data)))
        if end < 0:
            raise component.ComponentError("unbounded Windows client PE name")
        offset(rva, end - start + 1)
        return data[start:end]

    def directory(index: int) -> tuple[int, int]:
        return struct.unpack("<II", read(optional + 112 + index * 8, 8))

    export_rva, export_size = directory(0)
    if not export_rva or export_size < 40:
        raise component.ComponentError("Windows client lacks bootstrap exports")
    export = offset(export_rva, 40)
    count, names_rva = struct.unpack("<II", read(export + 24, 8))
    if not 1 <= count <= 4096:
        raise component.ComponentError("Windows client export count is unsupported")
    names = offset(names_rva, count * 4)
    if b"RunWinMain" not in [name(integer(names + index * 4)) for index in range(count)]:
        raise component.ComponentError("Windows client DLL does not export RunWinMain")
    imports_rva, imports_size = directory(1)
    if imports_rva:
        if not 20 <= imports_size <= 1024 * 1024:
            raise component.ComponentError("Windows client import directory exceeds its bound")
        imports = offset(imports_rva, imports_size)
        terminated = False
        for current in range(imports, imports + imports_size - 19, 20):
            descriptor = read(current, 20)
            if descriptor == bytes(20):
                terminated = True
                break
            if name(struct.unpack("<I", descriptor[12:16])[0]).lower() == b"libcef.dll":
                raise component.ComponentError("libcef.dll must be delay-loaded after sandbox bootstrap")
        if not terminated:
            raise component.ComponentError("Windows client import directory lacks its terminator")
    elif imports_size:
        raise component.ComponentError("Windows client import directory address is absent")
    delayed_rva, delayed_size = directory(13)
    if not delayed_rva or not 32 <= delayed_size <= 1024 * 1024:
        raise component.ComponentError("Windows client lacks bounded delayed imports")
    delayed = offset(delayed_rva, delayed_size)
    cef_delayed = False
    terminated = False
    for current in range(delayed, delayed + delayed_size - 31, 32):
        descriptor = read(current, 32)
        if descriptor == bytes(32):
            terminated = True
            break
        attributes, name_rva = struct.unpack("<II", descriptor[:8])
        if attributes != 1:
            raise component.ComponentError("Windows client delay imports must use RVAs")
        cef_delayed |= name(name_rva).lower() == b"libcef.dll"
    if not terminated:
        raise component.ComponentError("Windows client delay import directory lacks its terminator")
    if not cef_delayed:
        raise component.ComponentError("Windows client does not delay-load libcef.dll")


def grant_lpac_read(component_root: Path) -> None:
    if os.name != "nt":
        raise component.ComponentError("Windows sandbox ACL staging requires Windows")
    # CEF's SET_LPAC_ACLS requirement applies to immutable engine bytes only.
    # No user profile, credential, key or temporary home lives under this root.
    subprocess.run(["icacls.exe", os.fspath(component_root), "/grant",
                    "*S-1-15-2-2:(OI)(CI)(RX)"], check=True, timeout=30,
                   stdout=subprocess.DEVNULL)


def stage(cef_root: Path, native_build: Path, client_dll: Path, destination: Path,
          dictation_resources: Path | None = None, *, host: bool = False) -> Path:
    program = HOST_PROGRAM if host else PROGRAM
    if host and dictation_resources is not None:
        raise component.ComponentError("private browser host cannot contain Desktop dictation resources")
    source = stage_macos.verify_source(cef_root, "windows64")
    client_dll = client_dll.absolute()
    inspect_client(client_dll)
    helper = component.directory(native_build) / "colossus-browser-helper.dll"
    inspect_client(helper)
    destination = destination.absolute()
    parent = component.directory(destination.parent, create=True)
    if destination.exists() or destination.is_symlink():
        raise component.ComponentError("select a new Windows component destination")
    for name in ("bootstrap.exe", "chrome_elf.dll", "libcef.dll", "v8_context_snapshot.bin"):
        component.digest_file(source / "Release" / name)
    for name in RESOURCES:
        component.digest_file(source / "Resources" / name)
    if not (source / "Resources/locales").is_dir():
        raise component.ComponentError("CEF Windows locales are absent")
    with tempfile.TemporaryDirectory(prefix=".cef-windows-stage-", dir=parent) as temporary:
        staged = Path(temporary) / destination.name
        staged.mkdir()
        for directory in (source / "Release", source / "Resources"):
            for entry in directory.iterdir():
                if entry.name in ("bootstrap.exe", "bootstrapc.exe") or entry.suffix in (".lib", ".pdb"):
                    continue
                target = staged / entry.name
                if target.exists():
                    raise component.ComponentError("CEF Windows runtime/resource names collide")
                if entry.is_dir():
                    shutil.copytree(entry, target)
                else:
                    shutil.copy2(entry, target)
        shutil.copy2(source / "Release/bootstrap.exe", staged / (program + ".exe"))
        shutil.copy2(client_dll, staged / (program + ".dll"))
        shutil.copy2(source / "Release/bootstrap.exe", staged / "colossus-browser-helper.exe")
        shutil.copy2(helper, staged / "colossus-browser-helper.dll")
        notices = staged / "cef-notices"
        notices.mkdir()
        for name in ("LICENSE.txt", "CREDITS.html"):
            shutil.copy2(source / name, notices / name)
        if dictation_resources is not None:
            dictation = staged / "dictation"
            dictation.mkdir()
            for name in stage_macos.DICTATION_RESOURCES:
                component.digest_file(dictation_resources / name)
                shutil.copy2(dictation_resources / name, dictation / name)
        # Cargo/CMake may replace build outputs after the source inspection.
        # Validate the actual private copies that will be inventoried/published.
        inspect_client(staged / (program + ".dll"))
        inspect_client(staged / "colossus-browser-helper.dll")
        grant_lpac_read(staged)
        component.installed_inventory(staged, "windows64", program + ".exe")
        component.verify_installed(staged)
        if os.name == "nt":
            # Windows os.rename fails if any destination already exists.
            os.rename(staged, destination)
        else:
            stage_macos.rename_exclusive(staged, destination)
    return destination / (program + ".exe")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cef-root", type=Path, required=True)
    parser.add_argument("--client-dll", type=Path, required=True)
    parser.add_argument("--native-build", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--dictation-resources", type=Path)
    parser.add_argument("--host", action="store_true", help="stage the fixed private supervised host bootstrap")
    arguments = parser.parse_args()
    try:
        print(stage(arguments.cef_root, arguments.native_build, arguments.client_dll, arguments.destination,
                    arguments.dictation_resources, host=arguments.host))
        return 0
    except (component.ComponentError, OSError, subprocess.SubprocessError, ValueError) as error:
        print(f"Windows browser stage: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
