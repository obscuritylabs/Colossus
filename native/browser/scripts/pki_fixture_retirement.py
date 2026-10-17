"""Test-only source-input copies and the native pre-CEF retirement receipt.

Only disposable generated fixture material is copied here. Production retirement
is implemented by the supervisor's retained-inode StagedPki guard. Neither this
helper nor a successful TLS fixture establishes renderer/broker key custody.
"""

from __future__ import annotations

import ctypes
import os
from pathlib import Path
import stat
import sys
import uuid

from pki_fixture_private import private_directory
from run_probe import ProbeError


class StartupError(ProbeError):
    def __init__(self, message: str, phase: int):
        super().__init__(message)
        self.phase = phase


class FixtureInputs:
    def __init__(self, directory: Path):
        private_directory(directory)
        self.directory = directory
        self.fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        self.inputs = []
        self.quarantined = False

    def copy(self, source: Path, name: str, limit: int) -> str:
        if Path(name).name != name or name in ("", ".", ".."):
            raise ProbeError("native fixture input name rejected")
        incoming = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        outgoing = None
        try:
            metadata = os.fstat(incoming)
            if not self.private_file(metadata) or not 0 < metadata.st_size <= limit:
                raise ProbeError("native fixture input ownership or bounds rejected")
            outgoing = os.open(name, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW |
                               os.O_CLOEXEC, 0o600, dir_fd=self.fd)
            self.inputs.append({"name": name, "fd": outgoing, "retired": False})
            copied = 0
            while block := os.read(incoming, min(65536, limit + 1 - copied)):
                copied += len(block)
                if copied > limit:
                    raise ProbeError("native fixture input changed beyond its bound")
                view = memoryview(block)
                while view:
                    written = os.write(outgoing, view)
                    if written == 0:
                        raise ProbeError("native fixture input copy did not advance")
                    view = view[written:]
            settled = os.fstat(incoming)
            attributes = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns",
                          "st_uid", "st_mode", "st_nlink")
            if copied != metadata.st_size or any(getattr(settled, value) != getattr(metadata, value)
                                                  for value in attributes):
                raise ProbeError("native fixture input changed during its copy")
            os.fsync(outgoing)
            os.fsync(self.fd)
            return str(self.directory / name)
        finally:
            os.close(incoming)

    @staticmethod
    def private_file(metadata):
        return stat.S_ISREG(metadata.st_mode) and metadata.st_uid == os.geteuid() \
            and metadata.st_nlink == 1 and stat.S_IMODE(metadata.st_mode) & 0o077 == 0

    def retire(self) -> None:
        libc = ctypes.CDLL(None, use_errno=True)
        # Linux and macOS expose different no-replace rename operations. Both
        # must reject a pre-existing quarantine name atomically; plain renameat
        # could overwrite an unknown file before we compare the retained inode.
        if sys.platform == "linux":
            symbol, no_replace = "renameat2", 1  # RENAME_NOREPLACE
        elif sys.platform == "darwin":
            symbol, no_replace = "renameatx_np", 4  # RENAME_EXCL
        else:
            self.quarantined = True
            raise ProbeError("native fixture source retirement is unsupported on this OS")
        rename = getattr(libc, symbol, None)
        if rename is None:
            self.quarantined = True
            raise ProbeError("native fixture source retirement lacks atomic no-replace rename")
        rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p,
                           ctypes.c_uint]
        rename.restype = ctypes.c_int
        for item in self.inputs:
            if item["retired"]:
                continue
            held = os.fstat(item["fd"])
            if not self.private_file(held):
                self.quarantined = True
                raise ProbeError("native fixture source retirement ownership unknown")
            quarantine = f".retired-input-{uuid.uuid4().hex}"
            if rename(self.fd, os.fsencode(item["name"]), self.fd,
                      os.fsencode(quarantine), no_replace) != 0:
                self.quarantined = True
                raise ProbeError("native fixture source retirement could not quarantine its input")
            moved = os.stat(quarantine, dir_fd=self.fd, follow_symlinks=False)
            if not self.private_file(moved) or (moved.st_dev, moved.st_ino) != (held.st_dev, held.st_ino):
                self.quarantined = True
                raise ProbeError("native fixture source retirement preserved an unknown replacement")
            os.unlink(quarantine, dir_fd=self.fd)
            item["retired"] = True
        os.fsync(self.fd)
        for item in self.inputs:
            os.close(item["fd"])
        self.inputs.clear()

    def close(self) -> None:
        for item in self.inputs:
            os.close(item["fd"])
        self.inputs.clear()
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def startup_receipt(stream, inputs: FixtureInputs) -> None:
    """Require ordered native phases; retire sources before phase4 ACK."""
    previous = 0
    for _ in range(7):
        record = b""
        while len(record) < 5:
            block = stream.recv(5 - len(record))
            if not block:
                raise StartupError("native PKI startup receipt closed before initialization", previous)
            record += block
        if record[:4] != b"CBH\x01" or record[4] != previous + 1:
            raise StartupError("native PKI startup receipt rejected its phase", previous)
        previous = record[4]
        if previous == 4:
            inputs.retire()
            stream.sendall(b"CBH\x01\x84")
    if previous != 7:
        raise ProbeError("native PKI startup receipt omitted initialization")
