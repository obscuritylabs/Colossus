#!/usr/bin/env python3
"""Developer-only provisioning and inventory for the shipped CEF component.

The runtime never invokes this utility or downloads a browser. Sources are fixed
by the reviewed lockfile; installed inventories are bound by the host's signed
release manifest, rather than trusting this adjacent JSON as its own authority.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import shutil
import stat
import sys
import tarfile
import tempfile
import time
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, build_opener


LOCKFILE = Path(__file__).resolve().parent.parent / "component" / "cef.lock.json"
ORIGIN = "https://cef-builds.spotifycdn.com"
CEF_VERSION = "154.0.34+g14c5a08+chromium-154.0.8037.98"
CHROMIUM_VERSION = "154.0.8037.98"
MANIFEST = "browser-component.json"
SOURCE_MANIFEST = ".cef-source-manifest.json"
PLATFORMS = ("linux64", "windows64", "macosx64", "macosarm64")
MAX_FILES = 20_000
MAX_UNPACKED = 4 * 1024**3
MAX_MANIFEST_BYTES = 8 * 1024**2
MAX_REDIRECTS = 3
MAX_DOWNLOAD_SECONDS = 600


class ComponentError(ValueError):
    """A component cannot be safely provisioned or verified."""


def safe_path(value: str) -> str:
    """Accept one portable, unambiguous, relative POSIX path."""
    if not isinstance(value, str) or not value or "\\" in value or ":" in value:
        raise ComponentError(f"invalid relative path: {value!r}")
    if any(ord(char) < 32 for char in value):
        raise ComponentError("control character in path")
    path = PurePosixPath(value)
    parts = value.split("/")
    reserved = {"CON", "PRN", "AUX", "NUL"}
    reserved.update(f"COM{i}" for i in range(1, 10))
    reserved.update(f"LPT{i}" for i in range(1, 10))
    if path.is_absolute() or any(
        part in ("", ".", "..")
        or part.endswith((".", " "))
        or part.split(".", 1)[0].upper() in reserved
        for part in parts
    ):
        raise ComponentError(f"unsafe relative path: {value!r}")
    return value


def link_destination(name: str, target: str) -> str:
    if not target or "\\" in target or ":" in target:
        raise ComponentError(f"invalid link target at {name}")
    if target.startswith("/") or any(ord(char) < 32 for char in target):
        raise ComponentError(f"absolute or invalid link at {name}")
    destination = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
    return safe_path(destination)


def validate_links(kinds: dict[str, str], links: dict[str, str]) -> None:
    """Check the complete link graph, including links through directory prefixes."""
    for name in kinds:
        for parent in PurePosixPath(name).parents:
            if str(parent) in links:
                raise ComponentError(f"entry below symlink: {name}")
            if kinds.get(str(parent)) == "file":
                raise ComponentError(f"entry below regular file: {name}")

    def resolve(name: str, visiting: set[str]) -> str:
        parts = name.split("/")
        for index in range(1, len(parts) + 1):
            prefix = "/".join(parts[:index])
            if prefix in links:
                if prefix in visiting or len(visiting) >= 64:
                    raise ComponentError(f"symlink cycle or excessive nesting: {prefix}")
                destination = link_destination(prefix, links[prefix])
                suffix = parts[index:]
                if suffix:
                    destination += "/" + "/".join(suffix)
                return resolve(destination, visiting | {prefix})
        if name not in kinds:
            raise ComponentError(f"missing symlink destination: {name}")
        return name

    for name, target in links.items():
        resolve(link_destination(name, target), {name})


def digest_file(path: Path) -> str:
    """Hash a regular, singly linked file without following an introduced link."""
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
        raise ComponentError(f"expected an unlinked regular file: {path}")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0)
    descriptor = os.open(path, flags)
    with os.fdopen(descriptor, "rb") as source:
        opened = os.fstat(source.fileno())
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise ComponentError(f"file changed while opening: {path}")
        digest = hashlib.file_digest(source, "sha256").hexdigest()
        after = os.fstat(source.fileno())
    current = path.lstat()
    if any(
        (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        != (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
        for value in (after, current)
    ):
        raise ComponentError(f"file changed while hashing: {path}")
    return digest


def directory(path: Path, *, create: bool = False) -> Path:
    path = path.absolute()
    if create:
        path.mkdir(parents=True, exist_ok=True)
    if path.is_symlink() or not path.is_dir():
        raise ComponentError(f"expected a real directory: {path}")
    # Canonicalize parent aliases once, then retain this actual root for all work.
    return path.resolve(strict=True)


def load_json(path: Path) -> dict:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_MANIFEST_BYTES:
        raise ComponentError(f"missing, linked, or oversized manifest: {path}")

    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ComponentError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    with path.open(encoding="utf-8") as source:
        value = json.load(source, object_pairs_hook=unique_object)
    if not isinstance(value, dict):
        raise ComponentError("expected a JSON object")
    return value


def load_lock() -> dict:
    lock = load_json(LOCKFILE)
    if (
        lock.get("schema_version") != 1
        or lock.get("source_origin") != ORIGIN
        or lock.get("cef_version") != CEF_VERSION
        or lock.get("chromium_version") != CHROMIUM_VERSION
        or set(lock.get("archives", {})) != set(PLATFORMS)
    ):
        raise ComponentError("unsupported CEF source lock")
    for platform, archive in lock["archives"].items():
        expected_name = f"cef_binary_{CEF_VERSION}_{platform}_minimal.tar.bz2"
        digest = archive.get("sha256", "")
        if (
            archive.get("filename") != expected_name
            or not isinstance(digest, str)
            or len(digest) != 64
            or any(char not in "0123456789abcdef" for char in digest)
            or type(archive.get("size")) is not int
            or not 0 < archive["size"] <= 1024**3
        ):
            raise ComponentError(f"invalid archive pin for {platform}")
    return lock


def verify_archive(path: Path, pin: dict) -> None:
    if path.lstat().st_size != pin["size"] or digest_file(path) != pin["sha256"]:
        raise ComponentError(f"archive SHA-256 or size does not match reviewed pin: {path}")


class FixedOriginRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        parsed = urlsplit(new_url)
        original = urlsplit(request.full_url)
        if (
            parsed.scheme != "https"
            or parsed.netloc != "cef-builds.spotifycdn.com"
            or parsed.username is not None
            or parsed.path != original.path
            or parsed.query
            or parsed.fragment
        ):
            raise ComponentError("CEF source redirected outside its fixed HTTPS archive URL")
        redirects = getattr(request, "cef_redirects", 0) + 1
        if redirects > MAX_REDIRECTS:
            raise ComponentError("too many CEF source redirects")
        redirected = super().redirect_request(request, response, code, message, headers, new_url)
        if redirected is not None:
            redirected.cef_redirects = redirects
        return redirected


def fetch_archive(cache: Path, platform: str, pin: dict) -> Path:
    destination = cache / f"cef-{platform}.tar.bz2"
    if destination.exists() or destination.is_symlink():
        verify_archive(destination, pin)
        return destination
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".cef-{platform}-", dir=cache)
    temporary = Path(temporary_name)
    try:
        opener = build_opener(FixedOriginRedirects())
        deadline = time.monotonic() + MAX_DOWNLOAD_SECONDS
        with os.fdopen(descriptor, "wb") as output:
            with opener.open(f"{ORIGIN}/{pin['filename']}", timeout=60) as source:
                length = source.headers.get("Content-Length")
                if length is not None and int(length) != pin["size"]:
                    raise ComponentError("CEF source length differs from its reviewed pin")
                downloaded = 0
                while block := source.read(1024**2):
                    downloaded += len(block)
                    if time.monotonic() > deadline:
                        raise ComponentError("CEF download exceeded its total time limit")
                    if downloaded > pin["size"]:
                        raise ComponentError("CEF download exceeded its reviewed size")
                    output.write(block)
            output.flush()
            os.fsync(output.fileno())
        verify_archive(temporary, pin)
        if destination.exists() or destination.is_symlink():
            raise ComponentError("CEF cache destination appeared during download")
        temporary.rename(destination)
        return destination
    finally:
        temporary.unlink(missing_ok=True)


def extract_archive(archive: Path, destination: Path, expected_root: str) -> None:
    """Validate all members first, then extract without tarfile.extractall."""
    if destination.exists() or destination.is_symlink():
        raise ComponentError(f"refusing to replace existing extraction: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    parent = directory(destination.parent)
    with tempfile.TemporaryDirectory(prefix=".cef-extract-", dir=parent) as temporary:
        staging = Path(temporary)
        with tarfile.open(archive, mode="r:bz2") as source:
            members = []
            kinds: dict[str, str] = {}
            links: dict[str, str] = {}
            folded: set[str] = set()
            total = 0
            for member in source:
                members.append(member)
                if len(members) > MAX_FILES:
                    raise ComponentError("too many CEF archive entries")
                name = safe_path(member.name.removesuffix("/") if member.isdir() else member.name)
                if name.split("/", 1)[0] != expected_root:
                    raise ComponentError("CEF archive has an unexpected root")
                if name.casefold() in folded:
                    raise ComponentError(f"duplicate or case-colliding archive member: {name}")
                folded.add(name.casefold())
                if member.isdir():
                    kinds[name] = "directory"
                elif member.isfile():
                    kinds[name] = "file"
                    total += member.size
                    if member.size < 0 or total > MAX_UNPACKED:
                        raise ComponentError("CEF archive exceeds unpacked size limit")
                elif member.issym():
                    kinds[name] = "symlink"
                    links[name] = member.linkname
                    target = link_destination(name, member.linkname)
                    if target.split("/", 1)[0] != expected_root:
                        raise ComponentError(f"symlink escapes CEF root: {name}")
                else:
                    raise ComponentError(f"unsupported CEF archive entry: {name}")
            validate_links(kinds, links)
            for member in members:
                path = staging / member.name.removesuffix("/")
                if member.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                elif member.isfile():
                    path.parent.mkdir(parents=True, exist_ok=True)
                    input_file = source.extractfile(member)
                    if input_file is None:
                        raise ComponentError(f"missing archive data: {member.name}")
                    with input_file, path.open("xb") as output:
                        shutil.copyfileobj(input_file, output, length=1024**2)
                    if path.stat().st_size != member.size:
                        raise ComponentError(f"truncated archive entry: {member.name}")
                    path.chmod(0o755 if member.mode & 0o111 else 0o644)
            for name, target in links.items():
                path = staging / name
                path.parent.mkdir(parents=True, exist_ok=True)
                resolved_kind = kinds.get(link_destination(name, target))
                path.symlink_to(target, target_is_directory=resolved_kind != "file")
            for current, directories, _ in os.walk(staging, followlinks=False):
                Path(current).chmod(0o755)
                directories[:] = [name for name in directories if not (Path(current) / name).is_symlink()]
        if destination.exists() or destination.is_symlink():
            raise ComponentError("CEF extraction destination appeared during provisioning")
        (staging / expected_root).rename(destination)


def inventory_files(root: Path, excluded: str | None) -> list[dict]:
    records: list[dict] = []
    kinds: dict[str, str] = {}
    links: dict[str, str] = {}
    folded: set[str] = set()
    def inaccessible(error):
        raise error

    for current, directories, filenames in os.walk(root, followlinks=False, onerror=inaccessible):
        # os.walk places directory symlinks in directories; inventory and prune them.
        entries = sorted(directories + filenames)
        directories[:] = [name for name in directories if not (Path(current) / name).is_symlink()]
        for name in entries:
            path = Path(current) / name
            relative = safe_path(path.relative_to(root).as_posix())
            if relative == excluded:
                continue
            if relative.casefold() in folded:
                raise ComponentError(f"case-colliding inventory path: {relative}")
            folded.add(relative.casefold())
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                target = os.readlink(path)
                kinds[relative] = "symlink"
                links[relative] = target
                records.append({"path": relative, "kind": "symlink", "target": target,
                                "sha256": hashlib.sha256(target.encode("utf-8")).hexdigest()})
            elif stat.S_ISDIR(info.st_mode):
                kinds[relative] = "directory"
                records.append({"path": relative, "kind": "directory", "mode": stat.S_IMODE(info.st_mode)})
            elif stat.S_ISREG(info.st_mode):
                if info.st_mode & (stat.S_ISUID | stat.S_ISGID):
                    raise ComponentError(f"unexpected privileged component file: {relative}")
                kinds[relative] = "file"
                records.append({"path": relative, "kind": "file", "size": info.st_size,
                                "sha256": digest_file(path), "mode": stat.S_IMODE(info.st_mode)})
            else:
                raise ComponentError(f"unsupported installed component file: {relative}")
            if len(records) > MAX_FILES:
                raise ComponentError("too many installed component entries")
    validate_links(kinds, links)
    return sorted(records, key=lambda record: record["path"])


def source_archive_inventory(archive: Path, expected_root: str) -> list[dict]:
    """Derive cache authority from the pinned archive, never an adjacent receipt."""
    records = []
    with tarfile.open(archive, "r|bz2") as source:
        for member in source:
            name = safe_path(member.name.removesuffix("/") if member.isdir() else member.name)
            if name == expected_root and member.isdir():
                continue
            prefix = expected_root + "/"
            if not name.startswith(prefix):
                raise ComponentError("unexpected verified source archive root")
            relative = name.removeprefix(prefix)
            if member.isdir():
                record = {"path": relative, "kind": "directory", "mode": 0o755}
            elif member.isfile():
                data = source.extractfile(member)
                if data is None:
                    raise ComponentError("verified source archive has missing file data")
                with data:
                    digest = hashlib.file_digest(data, "sha256").hexdigest()
                record = {"path": relative, "kind": "file", "size": member.size,
                          "sha256": digest, "mode": 0o755 if member.mode & 0o111 else 0o644}
            elif member.issym():
                record = {"path": relative, "kind": "symlink", "target": member.linkname,
                          "sha256": hashlib.sha256(member.linkname.encode("utf-8")).hexdigest()}
            else:
                raise ComponentError("verified source archive has an unsupported entry")
            records.append(record)
    return sorted(records, key=lambda record: record["path"])


def source_inventory_matches(expected: list[dict], actual: object) -> bool:
    """Compare archived bytes/tree, using POSIX modes only where representable.

    Windows chmod/stat preserve only the read-only attribute of archive modes. DACL
    enforcement is a separate native requirement, never proven by these modes.
    Digests, sizes, paths, kinds and link destinations remain exact on all hosts.
    """
    if not isinstance(actual, list) or any(not isinstance(record, dict) for record in actual):
        return False
    if os.name != "nt":
        return expected == actual

    def comparable(record: dict) -> dict | None:
        normalized = record.copy()
        if record.get("kind") in ("file", "directory"):
            mode = record.get("mode")
            if not isinstance(mode, int) or isinstance(mode, bool):
                return None
            normalized["mode"] = bool(mode & stat.S_IWRITE)
        return normalized

    normalized_actual = [comparable(record) for record in actual]
    return None not in normalized_actual and [comparable(record) for record in expected] == normalized_actual


def write_manifest(root: Path, name: str, document: dict) -> Path:
    destination = root / name
    if destination.exists() or destination.is_symlink():
        raise ComponentError(f"refusing to replace existing inventory: {destination}")
    with destination.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(document, output, indent=2, sort_keys=True)
        output.write("\n")
    return destination


def provision(cache: Path, platform: str) -> Path:
    cache = directory(cache, create=True)
    cache_info = cache.stat()
    if os.name != "nt" and (cache_info.st_uid != os.getuid() or cache_info.st_mode & 0o022):
        raise ComponentError("CEF developer cache must be owned by this user and not writable by other users")
    pin = load_lock()["archives"][platform]
    archive = fetch_archive(cache, platform, pin)
    sources = directory(cache / "sources", create=True)
    source_root = sources / pin["filename"].removesuffix(".tar.bz2")
    if source_root.exists() or source_root.is_symlink():
        root = directory(source_root)
        receipt = load_json(root / SOURCE_MANIFEST)
        expected_files = source_archive_inventory(archive, source_root.name)
        if (
            receipt.get("schema_version") != 1
            or not source_inventory_matches(expected_files, receipt.get("files"))
            or receipt.get("archive_sha256") != pin["sha256"]
            or not source_inventory_matches(expected_files, inventory_files(root, SOURCE_MANIFEST))
        ):
            raise ComponentError("previous CEF extraction no longer matches its verified inventory")
        return root
    extract_archive(archive, source_root, source_root.name)
    write_manifest(source_root, SOURCE_MANIFEST, {
        "schema_version": 1, "archive_sha256": pin["sha256"],
        "files": inventory_files(source_root, SOURCE_MANIFEST),
    })
    return source_root


def external_manifest_path(root: Path, manifest_path: Path) -> Path:
    manifest_path = manifest_path.absolute()
    manifest_path = directory(manifest_path.parent) / manifest_path.name
    if manifest_path.is_relative_to(root):
        raise ComponentError("an external inventory must be outside the component root")
    return manifest_path


def installed_inventory(root: Path, platform: str, executable: str,
                        manifest_path: Path | None = None) -> Path:
    root = directory(root)
    executable = safe_path(executable)
    pin = load_lock()["archives"][platform]
    if manifest_path is not None:
        manifest_path = external_manifest_path(root, manifest_path)
    records = inventory_files(root, MANIFEST if manifest_path is None else None)
    verify_linux_host_link_order(root, platform, records)
    program = next((record for record in records if record["path"] == executable), None)
    if program is None or program["kind"] != "file":
        raise ComponentError("component executable must be an inventoried regular file")
    if platform != "windows64" and not program["mode"] & 0o111:
        raise ComponentError("component executable lacks executable permission")
    document = {
        "schema_version": 1, "protocol_version": 1, "component": "colossus-browser",
        "cef_version": CEF_VERSION, "chromium_version": CHROMIUM_VERSION,
        "archive_sha256": pin["sha256"], "platform": platform, "target": pin["target"],
        "executable": executable, "modes": {"desktop": False, "headless": False},
        "files": records,
    }
    if manifest_path is None:
        return write_manifest(root, MANIFEST, document)
    return write_manifest(manifest_path.parent, manifest_path.name, document)


def verify_installed(root: Path, manifest_path: Path | None = None) -> dict:
    """Verify a developer component without granting either production mode."""
    return _verify_installed(root, manifest_path, {"desktop": False, "headless": False})


def verify_publisher_accepted_inventory(root: Path, native_proof: dict) -> dict:
    """Recheck an accepted Linux inventory after compiler-sealed native verification.

    The package builder must obtain this proof by executing its exact sealed CLI.
    This does not change developer verification or promote a component manifest.
    """
    root = directory(root)
    if native_proof.get("publisher_acceptance_verified") is not True \
            or native_proof.get("payload_verified") is not True \
            or native_proof.get("component_manifest_sha256") != digest_file(root / MANIFEST):
        raise ComponentError("accepted inventory lacks its exact native publisher proof")
    document = _verify_installed(root, None, {"desktop": False, "headless": True})
    if document["platform"] != "linux64":
        raise ComponentError("accepted offline inventory requires the fixed Linux target")
    return document


def _verify_installed(root: Path, manifest_path: Path | None, expected_modes: dict) -> dict:
    root = directory(root)
    if manifest_path is not None:
        manifest_path = external_manifest_path(root, manifest_path)
    document = load_json(root / MANIFEST if manifest_path is None else manifest_path)
    if (
        document.get("schema_version") != 1 or document.get("protocol_version") != 1
        or document.get("component") != "colossus-browser"
        or document.get("cef_version") != CEF_VERSION
        or document.get("chromium_version") != CHROMIUM_VERSION
        or document.get("platform") not in PLATFORMS
        or document.get("modes") != expected_modes
    ):
        raise ComponentError("unsupported or unvalidated browser component manifest")
    pin = load_lock()["archives"][document["platform"]]
    if document.get("archive_sha256") != pin["sha256"] or document.get("target") != pin["target"]:
        raise ComponentError("installed component source pin or target differs from lock")
    files = inventory_files(root, MANIFEST if manifest_path is None else None)
    if document.get("files") != files:
        raise ComponentError("installed browser component inventory differs: missing, added, or modified entry")
    verify_linux_host_link_order(root, document["platform"], files)
    executable = safe_path(document.get("executable"))
    program = next((record for record in files if record["path"] == executable), None)
    if program is None or program["kind"] != "file":
        raise ComponentError("invalid inventoried browser executable")
    if document["platform"] != "windows64" and not program["mode"] & 0o111:
        raise ComponentError("installed browser executable lacks executable permission")
    return document


def verify_linux_host_link_order(root: Path, platform: str, records: list[dict]) -> None:
    """Require working libc interposition whenever the dedicated Linux host ships."""
    if platform != "linux64":
        return
    host = next((record for record in records if record["path"] == "colossus-native-browser-host"), None)
    if host is None:
        return
    if host["kind"] != "file" or not host["mode"] & 0o111:
        raise ComponentError("dedicated Linux browser host must be a regular executable")
    from elf_order import ElfOrderError, verify_chromium_link_order
    try:
        verify_chromium_link_order(root / "colossus-native-browser-host")
    except ElfOrderError as error:
        raise ComponentError(str(error)) from error


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    fetch = commands.add_parser("fetch", help="fetch and extract fixed pinned developer sources")
    fetch.add_argument("--platform", choices=PLATFORMS, required=True)
    fetch.add_argument("--cache", type=Path, default=Path(".local/cef-cache"))
    inventory = commands.add_parser("inventory", help="inventory a staged component with unvalidated modes")
    inventory.add_argument("--root", type=Path, required=True)
    inventory.add_argument("--platform", choices=PLATFORMS, required=True)
    inventory.add_argument("--executable", required=True)
    inventory.add_argument("--manifest", type=Path, help="external developer consistency inventory")
    verify = commands.add_parser("verify", help="verify every installed file and link")
    verify.add_argument("--root", type=Path, required=True)
    verify.add_argument("--manifest", type=Path, help="external developer consistency inventory")
    options = parser.parse_args()
    try:
        if options.command == "fetch":
            print(provision(options.cache, options.platform))
        elif options.command == "inventory":
            print(installed_inventory(options.root, options.platform, options.executable, options.manifest))
        else:
            document = verify_installed(options.root, options.manifest)
            print(json.dumps({"verified": True, "platform": document["platform"],
                              "protocol_version": document["protocol_version"],
                              "modes": document["modes"]}, sort_keys=True))
        return 0
    except (ComponentError, OSError, tarfile.TarError, ValueError) as error:
        print(f"browser component: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
