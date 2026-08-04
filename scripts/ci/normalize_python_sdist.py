#!/usr/bin/env python3
"""Rewrite a locally built Python sdist with deterministic archive metadata."""

from __future__ import annotations

import gzip
import io
import os
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


MAX_ARCHIVE_BYTES = 100 * 1024 * 1024
MAX_ARCHIVE_MEMBERS = 10_000


def fail(message: str) -> None:
    raise ValueError(message)


def safe_name(name: str) -> str:
    path = PurePosixPath(name)
    if path.is_absolute() or not name or ".." in path.parts:
        fail(f"unsafe Python sdist member: {name}")
    return name


def normalize_sdist(path: Path, source_date_epoch: int) -> None:
    if path.is_symlink():
        fail(f"Python sdist is a symbolic link: {path}")
    resolved = path.resolve(strict=True)
    if not resolved.is_file():
        fail(f"Python sdist is not a regular file: {resolved}")
    if resolved.stat().st_size > MAX_ARCHIVE_BYTES:
        fail(f"Python sdist exceeds {MAX_ARCHIVE_BYTES} bytes: {resolved}")

    entries: list[tuple[str, bool, bytes]] = []
    names: set[str] = set()
    total_bytes = 0
    with tarfile.open(resolved, mode="r:gz") as source:
        members = source.getmembers()
        if not members or len(members) > MAX_ARCHIVE_MEMBERS:
            fail(
                f"Python sdist must contain between 1 and {MAX_ARCHIVE_MEMBERS} members"
            )
        for member in members:
            name = safe_name(member.name)
            if name in names:
                fail(f"duplicate Python sdist member: {name}")
            names.add(name)
            if member.isdir():
                entries.append((name, True, b""))
            elif member.isfile():
                extracted = source.extractfile(member)
                if extracted is None:
                    fail(f"could not read Python sdist member: {name}")
                data = extracted.read(MAX_ARCHIVE_BYTES + 1)
                if len(data) > MAX_ARCHIVE_BYTES:
                    fail(
                        f"Python sdist member exceeds {MAX_ARCHIVE_BYTES} bytes: {name}"
                    )
                total_bytes += len(data)
                if total_bytes > MAX_ARCHIVE_BYTES:
                    fail(f"Python sdist contents exceed {MAX_ARCHIVE_BYTES} bytes")
                entries.append((name, False, data))
            else:
                fail(f"unsupported Python sdist member type: {name}")

    entries.sort(key=lambda entry: entry[0])
    with tempfile.TemporaryDirectory(
        prefix="colossus-sdist-", dir=resolved.parent
    ) as temp:
        temp_root = Path(temp)
        tar_path = temp_root / "normalized.tar"
        gzip_path = temp_root / resolved.name
        with tarfile.open(tar_path, mode="w", format=tarfile.PAX_FORMAT) as archive:
            for name, is_directory, data in entries:
                member = tarfile.TarInfo(name)
                member.type = tarfile.DIRTYPE if is_directory else tarfile.REGTYPE
                member.mode = 0o755 if is_directory else 0o644
                member.uid = 0
                member.gid = 0
                member.uname = ""
                member.gname = ""
                member.mtime = source_date_epoch
                member.pax_headers = {}
                member.size = 0 if is_directory else len(data)
                archive.addfile(member, None if is_directory else io.BytesIO(data))

        with tar_path.open("rb") as source, gzip_path.open("wb") as destination:
            with gzip.GzipFile(
                filename="",
                mode="wb",
                compresslevel=9,
                fileobj=destination,
                mtime=source_date_epoch,
            ) as compressed:
                shutil.copyfileobj(source, compressed)
        os.chmod(gzip_path, 0o644)
        os.replace(gzip_path, resolved)


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: normalize_python_sdist.py SDIST.tar.gz SOURCE_DATE_EPOCH")
    epoch_text = sys.argv[2]
    if (
        not epoch_text.isascii()
        or not epoch_text.isdecimal()
        or epoch_text.startswith("0")
    ):
        fail("SOURCE_DATE_EPOCH must be a positive integer")
    normalize_sdist(Path(sys.argv[1]), int(epoch_text))


if __name__ == "__main__":
    main()
