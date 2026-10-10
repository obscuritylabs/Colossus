"""Bounded x86-64 ELF dependency inspection without loading publisher code.

Chromium's exported close/localtime wrappers use RTLD_NEXT to reach libc.
The executable must load libcef before libc; otherwise CEF fatally terminates
even with an intact sandbox. This check is structural, not native acceptance.
"""
from __future__ import annotations

import os
from pathlib import Path
import stat
import struct

MAX_EXECUTABLE = 256 * 1024 * 1024
MAX_PROGRAM_HEADERS = 256
MAX_DYNAMIC = 64 * 1024
MAX_STRINGS = 1024 * 1024


class ElfOrderError(Exception):
    """Categorical invalid ELF or unusable Chromium dependency ordering."""


def _read(stream, offset: int, length: int, size: int) -> bytes:
    if offset < 0 or length < 0 or offset + length > size:
        raise ElfOrderError("Linux browser ELF range is invalid")
    stream.seek(offset)
    value = stream.read(length)
    if len(value) != length:
        raise ElfOrderError("Linux browser ELF range is incomplete")
    return value


def _dependencies(stream, size: int) -> list[str]:
    header = _read(stream, 0, 64, size)
    fields = struct.unpack("<16sHHIQQQIHHHHHH", header)
    if fields[0][:7] != b"\x7fELF\x02\x01\x01" or fields[1] not in (2, 3) \
            or fields[2] != 62 or fields[3] != 1 or fields[8] != 64 \
            or fields[9] != 56 or not 0 < fields[10] <= MAX_PROGRAM_HEADERS:
        raise ElfOrderError("Linux browser requires a bounded x86-64 ELF executable")
    program = _read(stream, fields[5], fields[9] * fields[10], size)
    loads, dynamic = [], []
    for offset in range(0, len(program), 56):
        kind, _, file_offset, address, _, file_size, memory_size, _ = struct.unpack_from(
            "<IIQQQQQQ", program, offset)
        if file_size > memory_size or file_offset + file_size > size \
                or address + memory_size > 2**64:
            raise ElfOrderError("Linux browser ELF segment is invalid")
        if kind == 1:
            loads.append((address, file_offset, file_size))
        elif kind == 2:
            dynamic.append((file_offset, file_size))
    if len(dynamic) != 1 or not 0 < dynamic[0][1] <= MAX_DYNAMIC or dynamic[0][1] % 16:
        raise ElfOrderError("Linux browser ELF dynamic table is invalid")
    table = _read(stream, *dynamic[0], size)
    needed, strings, string_sizes, terminated = [], [], [], False
    for offset in range(0, len(table), 16):
        kind, value = struct.unpack_from("<QQ", table, offset)
        if kind == 0:
            terminated = True
            break
        if kind == 1:
            needed.append(value)
        elif kind == 5:
            strings.append(value)
        elif kind == 10:
            string_sizes.append(value)
    if not terminated or not 0 < len(needed) <= MAX_PROGRAM_HEADERS \
            or len(strings) != 1 or len(string_sizes) != 1 \
            or not 0 < string_sizes[0] <= MAX_STRINGS:
        raise ElfOrderError("Linux browser ELF dependency metadata is invalid")
    address, length = strings[0], string_sizes[0]
    offsets = [file_offset + address - start for start, file_offset, file_size in loads
               if start <= address and address + length <= start + file_size]
    if len(offsets) != 1:
        raise ElfOrderError("Linux browser ELF string table is not uniquely file-backed")
    names = _read(stream, offsets[0], length, size)
    result = []
    for offset in needed:
        end = names.find(b"\0", offset, min(offset + 257, length))
        if offset >= length or end <= offset:
            raise ElfOrderError("Linux browser ELF dependency name is invalid")
        try:
            name = names[offset:end].decode("ascii")
        except UnicodeDecodeError as error:
            raise ElfOrderError("Linux browser ELF dependency name is invalid") from error
        if any(character not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._+-"
               for character in name) or name in result:
            raise ElfOrderError("Linux browser ELF dependency name is invalid")
        result.append(name)
    return result


def verify_chromium_link_order(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or not 64 <= metadata.st_size <= MAX_EXECUTABLE:
            raise ElfOrderError("Linux browser ELF executable is invalid or oversized")
        dependencies = _dependencies(stream, metadata.st_size)
    if "libcef.so" not in dependencies or "libc.so.6" not in dependencies \
            or dependencies.index("libcef.so") > dependencies.index("libc.so.6"):
        raise ElfOrderError("Linux browser requires libcef before libc for Chromium native interposers")
