"""Owner-private, newly allocated material for disposable browser PKI fixtures."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import stat


def private_directory(path: Path) -> None:
    """Create exactly one new directory; never change an existing ACL/mode."""
    if os.name != "nt":
        path.mkdir(mode=0o700)
        if stat.S_IMODE(path.stat().st_mode) != 0o700:
            raise ValueError("fixture directory is not owner-private")
        return
    # Supply the protected owner-only inheritable DACL at allocation, before any
    # key is written. chmod and inherited parent ACLs do not protect Windows keys.
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    security = ctypes.WinDLL("advapi32", use_last_error=True)

    class SecurityAttributes(ctypes.Structure):
        _fields_ = [("length", wintypes.DWORD), ("descriptor", wintypes.LPVOID),
                    ("inherit", wintypes.BOOL)]

    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.LocalFree.argtypes = [wintypes.HLOCAL]
    kernel.LocalFree.restype = wintypes.HLOCAL
    kernel.CreateDirectoryW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(SecurityAttributes)]
    kernel.CreateDirectoryW.restype = wintypes.BOOL
    security.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD,
                                         ctypes.POINTER(wintypes.HANDLE)]
    security.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int,
        wintypes.LPVOID, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
    security.ConvertSidToStringSidW.argtypes = [wintypes.LPVOID,
                                               ctypes.POINTER(wintypes.LPWSTR)]
    security.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(wintypes.LPVOID),
        ctypes.POINTER(wintypes.DWORD)]
    token = wintypes.HANDLE()
    sid_text = wintypes.LPWSTR()
    descriptor = wintypes.LPVOID()
    try:
        if not security.OpenProcessToken(kernel.GetCurrentProcess(), 0x0008,
                                         ctypes.byref(token)):
            raise ValueError("fixture owner token unavailable")
        length = wintypes.DWORD()
        security.GetTokenInformation(token, 1, None, 0, ctypes.byref(length))
        if not 0 < length.value <= 65536:
            raise ValueError("fixture owner token is invalid")
        information = ctypes.create_string_buffer(length.value)
        if not security.GetTokenInformation(token, 1, information, length,
                                             ctypes.byref(length)):
            raise ValueError("fixture owner token unavailable")
        sid = ctypes.cast(information, ctypes.POINTER(wintypes.LPVOID))[0]
        if not security.ConvertSidToStringSidW(sid, ctypes.byref(sid_text)):
            raise ValueError("fixture owner identity unavailable")
        sddl = f"D:P(A;OICI;FA;;;{sid_text.value})"
        if not security.ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl, 1, ctypes.byref(descriptor), None):
            raise ValueError("fixture private ACL unavailable")
        attributes = SecurityAttributes(ctypes.sizeof(SecurityAttributes), descriptor, False)
        if not kernel.CreateDirectoryW(str(path), ctypes.byref(attributes)):
            raise ValueError("new private fixture directory could not be allocated")
    finally:
        if descriptor:
            kernel.LocalFree(descriptor)
        if sid_text:
            kernel.LocalFree(ctypes.cast(sid_text, wintypes.HLOCAL))
        if token:
            kernel.CloseHandle(token)


def private_write(path: Path, data: bytes) -> None:
    """Publish one new regular fixture file without overwriting another entry."""
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                         getattr(os, "O_BINARY", 0), 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)
