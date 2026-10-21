"""Explicit Windows fixture custody: new CNG keys and exact public-cert entries.

Never enumerates, exports, overwrites or deletes another identity. The caller must
retain this owner through native browser shutdown and invoke close in finally.
"""
from __future__ import annotations

import ctypes
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import struct
import threading
import time
import uuid

from cryptography.hazmat.primitives.asymmetric import rsa
from pki_fixture_private import private_write

PROVIDER = "Microsoft Software Key Storage Provider"
NOT_FOUND = (0x80090011, 0x80090016)


class Blob(ctypes.Structure):
    _fields_ = [("length", w.DWORD), ("bytes", ctypes.c_void_p)]


class Certificate(ctypes.Structure):
    _fields_ = [("encoding", w.DWORD), ("bytes", ctypes.c_void_p),
                ("length", w.DWORD), ("info", ctypes.c_void_p), ("store", w.HANDLE)]


class ProviderInfo(ctypes.Structure):
    _fields_ = [("container", w.LPWSTR), ("provider", w.LPWSTR),
                ("kind", w.DWORD), ("flags", w.DWORD), ("count", w.DWORD),
                ("parameters", ctypes.c_void_p), ("key_spec", w.DWORD)]


class FixtureStore:
    def __init__(self, directory: Path, fixture_id: str, *, confirm_generated_roots=False):
        if str(uuid.UUID(fixture_id)) != fixture_id:
            raise ValueError("fixture UUID required")
        self.directory, self.fixture_id = directory, fixture_id
        self.confirm_generated_roots = confirm_generated_roots
        self.keys, self.certificates, self.history = [], [], []
        self.closed, self.cleanup_verified = False, False
        self.ncrypt = ctypes.WinDLL("ncrypt", use_last_error=True)
        self.crypto = ctypes.WinDLL("crypt32", use_last_error=True)
        h = ctypes.c_size_t
        for name, arguments in {
            "NCryptOpenStorageProvider": [ctypes.POINTER(h), w.LPCWSTR, w.DWORD],
            "NCryptOpenKey": [h, ctypes.POINTER(h), w.LPCWSTR, w.DWORD, w.DWORD],
            "NCryptCreatePersistedKey": [h, ctypes.POINTER(h), w.LPCWSTR, w.LPCWSTR, w.DWORD, w.DWORD],
            "NCryptSetProperty": [h, w.LPCWSTR, ctypes.c_void_p, w.DWORD, w.DWORD],
            "NCryptGetProperty": [h, w.LPCWSTR, ctypes.c_void_p, w.DWORD, ctypes.POINTER(w.DWORD), w.DWORD],
            "NCryptFinalizeKey": [h, w.DWORD],
            "NCryptExportKey": [h, h, w.LPCWSTR, ctypes.c_void_p, ctypes.c_void_p, w.DWORD, ctypes.POINTER(w.DWORD), w.DWORD],
            "NCryptDeleteKey": [h, w.DWORD], "NCryptFreeObject": [h],
        }.items():
            function = getattr(self.ncrypt, name)
            function.argtypes, function.restype = arguments, w.LONG
        cert = ctypes.POINTER(Certificate)
        for name, arguments, result in (
            ("CertOpenStore", [ctypes.c_void_p, w.DWORD, h, w.DWORD, ctypes.c_void_p], w.HANDLE),
            ("CertCloseStore", [w.HANDLE, w.DWORD], w.BOOL),
            ("CertCreateCertificateContext", [w.DWORD, ctypes.c_void_p, w.DWORD], cert),
            ("CertFreeCertificateContext", [cert], w.BOOL),
            ("CertSetCertificateContextProperty", [cert, w.DWORD, w.DWORD, ctypes.c_void_p], w.BOOL),
            ("CertAddCertificateContextToStore", [w.HANDLE, cert, w.DWORD, ctypes.c_void_p], w.BOOL),
            ("CertFindCertificateInStore", [w.HANDLE, w.DWORD, w.DWORD, w.DWORD, ctypes.c_void_p, cert], cert),
            ("CertDeleteCertificateFromStore", [cert], w.BOOL),
        ):
            function = getattr(self.crypto, name)
            function.argtypes, function.restype = arguments, result
        self.provider = h()
        self._ok(self.ncrypt.NCryptOpenStorageProvider(ctypes.byref(self.provider), PROVIDER, 0))
        try:
            self._record("preflight")
        except BaseException:
            self.ncrypt.NCryptFreeObject(self.provider)
            self.provider.value = 0
            raise

    @staticmethod
    def _ok(status):
        if status != 0:
            raise ValueError("owned Windows fixture key operation failed")

    def _record(self, phase):
        self.history.append(phase)
        ledger = {"schema_version": 1, "fixture_id": self.fixture_id,
                  "keys": [record for _, record in self.keys],
                  "certificates": [{"store": store, "sha256": hashlib.sha256(der).hexdigest(),
                                    "sha1": hashlib.sha1(der).hexdigest()}
                                   for store, der in self.certificates], "phases": self.history}
        # Each immutable checkpoint is newly allocated; no existing ledger is overwritten.
        private_write(self.directory / f"store-ledger-{len(self.history):03}.json",
                      (json.dumps(ledger, indent=2) + "\n").encode())

    def create_key(self, label: str):
        if label not in ("client", "alternate_client", "wrong_issuer_client"):
            raise ValueError("closed fixture key label required")
        name = f"Colossus-PKI-Fixture-{self.fixture_id}-{label}"
        handle = ctypes.c_size_t()
        existing = ctypes.c_size_t()
        status = self.ncrypt.NCryptOpenKey(self.provider, ctypes.byref(existing), name, 0, 0)
        if status == 0:
            self.ncrypt.NCryptFreeObject(existing)
            raise ValueError("fixture key name already exists; refusing adoption")
        if status & 0xffffffff not in NOT_FOUND:
            raise ValueError("fixture key absence could not be established")
        record = {"name": name, "provider": PROVIDER, "created": False, "public_blob_sha256": None}
        self.keys.append((handle, record))
        self._record("key_absent_before_create")
        # No MACHINE or OVERWRITE flags; the named user key must be new.
        self._ok(self.ncrypt.NCryptCreatePersistedKey(self.provider, ctypes.byref(handle), "RSA", name, 0, 0))
        record["created"] = True
        for property_name, value in (("Length", 2048), ("Export Policy", 0), ("Key Usage", 2)):
            data = w.DWORD(value)
            self._ok(self.ncrypt.NCryptSetProperty(handle, property_name, ctypes.byref(data), ctypes.sizeof(data), 0))
        self._ok(self.ncrypt.NCryptFinalizeKey(handle, 0))
        blob = self._public_blob(handle)
        record["public_blob_sha256"] = hashlib.sha256(blob).hexdigest()
        policy, size = w.DWORD(), w.DWORD()
        self._ok(self.ncrypt.NCryptGetProperty(handle, "Export Policy", ctypes.byref(policy), 4, ctypes.byref(size), 0))
        if size.value != 4 or policy.value != 0:
            raise ValueError("fixture private key must remain nonexportable")
        magic, bits, exponent_length, modulus_length, prime1, prime2 = struct.unpack("<6I", blob[:24])
        if (magic, bits, modulus_length, prime1, prime2) != (0x31415352, 2048, 256, 0, 0) or len(blob) != 24 + exponent_length + modulus_length:
            raise ValueError("fixture public RSA blob was invalid")
        public = rsa.RSAPublicNumbers(int.from_bytes(blob[24:24 + exponent_length], "big"),
                                     int.from_bytes(blob[24 + exponent_length:], "big")).public_key()
        self._record("nonexportable_user_key_created")
        return name, public

    def _public_blob(self, handle):
        size = w.DWORD()
        self._ok(self.ncrypt.NCryptExportKey(handle, 0, "RSAPUBLICBLOB", None, None, 0, ctypes.byref(size), 0))
        if not 24 < size.value <= 4096:
            raise ValueError("fixture public key exceeded bounds")
        buffer = ctypes.create_string_buffer(size.value)
        self._ok(self.ncrypt.NCryptExportKey(handle, 0, "RSAPUBLICBLOB", None, buffer, size, ctypes.byref(size), 0))
        return buffer.raw[:size.value]

    def _store(self, name):
        if name not in ("ROOT", "MY"):
            raise ValueError("closed fixture certificate store required")
        # CurrentUser, existing system store only; no LocalMachine or store creation.
        store = self.crypto.CertOpenStore(ctypes.c_void_p(10), 0, 0, 0x10000 | 0x4000,
                                          ctypes.cast(ctypes.c_wchar_p(name), ctypes.c_void_p))
        if not store:
            raise ValueError("fixture user certificate store could not be opened")
        return store

    def _find(self, store, der):
        digest = ctypes.create_string_buffer(hashlib.sha1(der).digest())
        blob = Blob(20, ctypes.cast(digest, ctypes.c_void_p))
        certificate = self.crypto.CertFindCertificateInStore(store, 1, 0, 0x10000, ctypes.byref(blob), None)
        if certificate and ctypes.string_at(certificate.contents.bytes, certificate.contents.length) != der:
            self.crypto.CertFreeCertificateContext(certificate)
            raise ValueError("fixture thumbprint matches different certificate; preserving store")
        if not certificate and ctypes.get_last_error() & 0xffffffff != 0x80092004:
            raise ValueError(f"fixture certificate absence could not be established ({ctypes.get_last_error() & 0xffffffff:08x})")
        return certificate

    def add_certificate(self, name: str, der: bytes, key_name=None):
        if not 0 < len(der) <= 65536 or (name == "ROOT") != (key_name is None):
            raise ValueError("fixture certificate shape invalid")
        store = self._store(name)
        context = None
        try:
            existing = self._find(store, der)
            if existing:
                self.crypto.CertFreeCertificateContext(existing)
                raise ValueError("fixture certificate already exists; refusing adoption")
            self.certificates.append((name, der))
            self._record("certificate_absent_before_add")
            encoded = ctypes.create_string_buffer(der)
            context = self.crypto.CertCreateCertificateContext(1, encoded, len(der))
            if not context:
                raise ValueError("fixture certificate invalid")
            if key_name is not None:
                if not any(record["name"] == key_name and record["created"] for _, record in self.keys):
                    raise ValueError("fixture owns no matching key")
                info = ProviderInfo(key_name, PROVIDER, 0, 0, 0, None, 0xffffffff)
                if not self.crypto.CertSetCertificateContextProperty(context, 2, 0, ctypes.byref(info)):
                    raise ValueError("fixture key association failed")
            # ADD_NEW never overwrites any existing certificate or its properties.
            if not self._root_operation(name, der, lambda: self.crypto.CertAddCertificateContextToStore(store, context, 1, None)):
                raise ValueError("fixture certificate import failed")
            self._record("fixture_certificate_added")
        finally:
            if context:
                self.crypto.CertFreeCertificateContext(context)
            self.crypto.CertCloseStore(store, 0)

    def close(self):
        if self.closed:
            if not self.cleanup_verified:
                raise ValueError("owned Windows PKI fixture cleanup remains unverified")
            return
        errors = False
        for name, der in reversed(self.certificates):
            store = None
            try:
                store = self._store(name)
                certificate = self._find(store, der)
                if certificate and not self._root_operation(name, der, lambda: self.crypto.CertDeleteCertificateFromStore(certificate)):
                    raise ValueError("fixture certificate cleanup failed")
                remaining = self._find(store, der)
                if remaining:
                    self.crypto.CertFreeCertificateContext(remaining)
                    raise ValueError("fixture certificate cleanup unverified")
            except (ValueError, OSError):
                errors = True
            finally:
                if store and not self.crypto.CertCloseStore(store, 0):
                    errors = True
        for handle, record in reversed(self.keys):
            try:
                if not record["created"] or not handle.value:
                    continue
                if record["public_blob_sha256"] is not None and hashlib.sha256(self._public_blob(handle)).hexdigest() != record["public_blob_sha256"]:
                    raise ValueError("owned fixture key no longer matches; preserving it")
                self._ok(self.ncrypt.NCryptDeleteKey(handle, 0))
                handle.value = 0
                existing = ctypes.c_size_t()
                status = self.ncrypt.NCryptOpenKey(self.provider, ctypes.byref(existing), record["name"], 0, 0)
                if status == 0:
                    self.ncrypt.NCryptFreeObject(existing)
                    raise ValueError("owned fixture key deletion unverified")
                if status & 0xffffffff not in NOT_FOUND:
                    raise ValueError("owned fixture key absence unverified")
            except (ValueError, OSError):
                errors = True
            finally:
                if handle.value:
                    self.ncrypt.NCryptFreeObject(handle)
                    handle.value = 0
        try:
            self._record("cleanup_unknown" if errors else "cleanup_verified")
            self.cleanup_verified = not errors
        finally:
            self.closed = True
            if self.provider.value:
                self.ncrypt.NCryptFreeObject(self.provider)
                self.provider.value = 0
        if errors:
            raise ValueError("owned Windows PKI fixture cleanup remains unverified")

    def _root_operation(self, name, der, operation):
        if name != "ROOT" or not self.confirm_generated_roots:
            return operation()
        # Explicit operator test only: acknowledge just this process's root
        # install/removal warning, with both unique UUID and exact public SHA1.
        digest = hashlib.sha1(der).hexdigest().upper()
        expected = " ".join(digest[index:index + 8] for index in range(0, 40, 8))
        stop = threading.Event()
        user = ctypes.WinDLL("user32", use_last_error=True)
        callback = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
        user.EnumWindows.argtypes = [callback, w.LPARAM]
        user.EnumChildWindows.argtypes = [w.HWND, callback, w.LPARAM]
        user.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
        user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
        user.GetDlgCtrlID.argtypes = [w.HWND]
        user.SendMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]

        def watch():
            deadline = time.monotonic() + 10
            @callback
            def inspect(window, _):
                process = w.DWORD()
                user.GetWindowThreadProcessId(window, ctypes.byref(process))
                if process.value != os.getpid():
                    return True
                children = []
                @callback
                def child(handle, _):
                    text = ctypes.create_unicode_buffer(4096)
                    user.GetWindowTextW(handle, text, len(text))
                    children.append((handle, user.GetDlgCtrlID(handle), text.value))
                    return True
                user.EnumChildWindows(window, child, 0)
                text = " ".join(text for _, _, text in children)
                buttons = [handle for handle, identifier, _ in children if identifier == 6]
                if self.fixture_id in text and expected in text and len(buttons) == 1:
                    user.SendMessageW(buttons[0], 0x00f5, 0, 0)
                    stop.set()
                return True
            while not stop.is_set() and time.monotonic() < deadline:
                user.EnumWindows(inspect, 0)
                stop.wait(0.02)
        worker = threading.Thread(target=watch, name="owned-root-fixture-confirmation")
        worker.start()
        try:
            return operation()
        finally:
            stop.set()
            worker.join(timeout=2)
            if worker.is_alive():
                raise ValueError("fixture root-confirmation worker did not settle")
