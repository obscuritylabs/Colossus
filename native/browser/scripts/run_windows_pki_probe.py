#!/usr/bin/env python3
"""Explicit local CEF PKI login acceptance with new Windows fixture keys only.

Requires --confirm-fixture-store-changes. New CurrentUser ROOT/MY entries and
nonexportable CNG keys are recorded before use and removed in finally. A bounded
native test driver answers only the generated origin/fingerprint dialog in the
selected component's new process; it never selects a personal identity.
"""
from __future__ import annotations
import argparse
import ctypes
from ctypes import wintypes as w
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import threading
import time

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID
from pki_fixture import Fixture
from pki_fixture_private import private_write
from windows_pki_store import FixtureStore


class DialogDriver:
    def __init__(self, directory, metadata, component):
        self.directory, self.metadata, self.component = directory, metadata, component
        self.case, self.clicks, self.seen, self.readonly_verified = None, 0, set(), False
        self.user = ctypes.WinDLL("user32", use_last_error=True)
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.callback = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
        for name, args in {
            "EnumWindows": [self.callback, w.LPARAM], "EnumChildWindows": [w.HWND, self.callback, w.LPARAM],
            "GetWindowThreadProcessId": [w.HWND, ctypes.POINTER(w.DWORD)],
            "GetWindowTextW": [w.HWND, w.LPWSTR, ctypes.c_int], "GetDlgCtrlID": [w.HWND],
            "GetWindowLongPtrW": [w.HWND, ctypes.c_int],
            "SendMessageTimeoutW": [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT, w.UINT, ctypes.POINTER(ctypes.c_size_t)],
        }.items():
            getattr(self.user, name).argtypes = args
        self.user.GetWindowLongPtrW.restype = ctypes.c_ssize_t
        self.kernel.OpenProcess.argtypes, self.kernel.OpenProcess.restype = [w.DWORD, w.BOOL, w.DWORD], w.HANDLE
        self.kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
        self.kernel.CloseHandle.argtypes = [w.HANDLE]

    def text(self, window):
        text = ctypes.create_unicode_buffer(8192)
        self.user.GetWindowTextW(window, text, len(text))
        return text.value

    def poll(self):
        if not self.case or (self.case == "cancelled_request" and self.case in self.seen):
            # The first fully validated prompt publishes the marker below.
            # Core then cancels the run; its closing dialog is no longer a
            # live consent surface to inspect or operate.
            return
        def inspect(window, _):
            if self.text(window) != "Colossus browser client certificate":
                return True
            process = w.DWORD()
            self.user.GetWindowThreadProcessId(window, ctypes.byref(process))
            handle = self.kernel.OpenProcess(0x1000, False, process.value)
            if not handle:
                return True
            try:
                image, length = ctypes.create_unicode_buffer(32768), w.DWORD(32768)
                if not self.kernel.QueryFullProcessImageNameW(handle, 0, image, ctypes.byref(length)):
                    return True
                if Path(image.value).resolve() != (self.component / "colossus-native-browser-host.exe").resolve():
                    return True
            finally:
                self.kernel.CloseHandle(handle)
            children = {}
            @self.callback
            def child(control, _):
                children[self.user.GetDlgCtrlID(control)] = control
                return True
            self.user.EnumChildWindows(window, child, 0)
            if not all(identifier in children for identifier in (1, 2, 1002, 1003, 1004)):
                return True
            origin, fingerprint = self.text(children[1003]), self.text(children[1004])
            if origin not in self.metadata["urls"].values() or fingerprint not in (
                    self.metadata["fingerprints_sha256"]["client"], self.metadata["fingerprints_sha256"]["alternate_client"]):
                raise ValueError("native review contained a non-fixture identity or origin")
            if not all(self.user.GetWindowLongPtrW(children[index], -16) & 0x800 for index in (1003, 1004)):
                raise ValueError("native origin/fingerprint must be read-only")
            default = ctypes.c_size_t()
            self.user.SendMessageTimeoutW(window, 0x400, 0, 0, 2, 1000, ctypes.byref(default))
            if default.value & 0xffff != 2:
                raise ValueError("native review must default to Cancel")
            self.readonly_verified = True
            self.seen.add(self.case)
            if self.case == "cancelled_request":
                marker = self.directory / "cancelled-request-dialog-seen"
                if not marker.exists():
                    private_write(marker, b"observed\n")
                return True
            desired = "alternate_client" if self.case == "alternate_identity" else "client"
            deny = self.case == "missing_identity" or (self.case == "changed_origin" and origin == self.metadata["urls"]["redirect_mtls"])
            identifier = 2 if deny else (1 if fingerprint == self.metadata["fingerprints_sha256"][desired] else 1002)
            self.clicks += 1
            if self.clicks > 32:
                raise ValueError("native fixture dialog action bound exceeded")
            result = ctypes.c_size_t()
            if not self.user.SendMessageTimeoutW(children[identifier], 0xf5, 0, 0, 2, 1000, ctypes.byref(result)):
                raise ValueError("owned native dialog did not acknowledge its fixed button")
            return True
        failures = []
        @self.callback
        def guarded(window, parameter):
            try:
                return inspect(window, parameter)
            except (ValueError, OSError) as error:
                failures.append(str(error))
                return False
        self.user.EnumWindows(guarded, 0)
        if failures:
            raise ValueError(failures[0])


def provision(fixture, owner):
    directory, metadata = fixture.directory, fixture.metadata
    ca = x509.load_pem_x509_certificate((directory / "ca.pem").read_bytes())
    ca_key = serialization.load_pem_private_key((directory / "ca.key").read_bytes(), None)
    now = datetime.now(timezone.utc)
    for label in ("client", "alternate_client"):
        name, public = owner.create_key(label)
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, f"Colossus {label} {metadata['fixture_id']}")])
        certificate = (x509.CertificateBuilder().subject_name(subject).issuer_name(ca.subject).public_key(public)
            .serial_number(x509.random_serial_number()).not_valid_before(now - timedelta(minutes=5))
            .not_valid_after(now + timedelta(hours=1)).add_extension(x509.BasicConstraints(False, None), True)
            .add_extension(x509.KeyUsage(True, False, False, False, False, False, False, None, None), True)
            .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.CLIENT_AUTH]), False)
            .sign(ca_key, hashes.SHA256()))
        der = certificate.public_bytes(serialization.Encoding.DER)
        metadata["fingerprints_sha256"][label] = hashlib.sha256(der).hexdigest()
        owner.add_certificate("MY", der, name)
    owner.add_certificate("ROOT", ca.public_bytes(serialization.Encoding.DER))


def run(arguments):
    fixture = Fixture(arguments.directory)
    owner = None
    result = None
    child = None
    server = None
    try:
        owner = FixtureStore(fixture.directory, fixture.metadata["fixture_id"], confirm_generated_roots=True)
        provision(fixture, owner)
        private_write(fixture.directory / "fixture-material.json", (json.dumps(fixture.metadata)+"\n").encode())
        server = subprocess.Popen([str(arguments.server_executable), str(fixture.directory)], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")
        startup = queue.Queue(maxsize=1)
        threading.Thread(target=lambda: startup.put(server.stdout.readline(16385)), daemon=True).start()
        try:
            ready = json.loads(startup.get(timeout=10))
        except queue.Empty as error:
            raise ValueError("local native TLS server startup deadline expired") from error
        if not ready.get("ready") or set(ready.get("urls", {})) != {"public", "mtls", "redirect_mtls", "untrusted", "wrong_hostname", "expired"}:
            raise ValueError("bounded local native TLS server did not become ready")
        metadata = dict(fixture.metadata, urls=ready["urls"])
        private_write(fixture.directory / "fixture.json", (json.dumps(metadata)+"\n").encode())
        driver = DialogDriver(fixture.directory, metadata, arguments.component)
        environment = dict(os.environ, COLOSSUS_WINDOWS_PKI_FIXTURE=str(fixture.directory),
            COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT=str(arguments.component),
            COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT=str(arguments.state_parent))
        child = subprocess.Popen([str(arguments.test_executable), "--ignored", "--exact", arguments.test,
            "--nocapture", "--test-threads=1"], env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace")
        lines = queue.Queue(maxsize=1024)
        def read():
            for line in child.stdout:
                lines.put(line[:4096])
            lines.put(None)
        reader = threading.Thread(target=read, name="bounded-PKI-test-output", daemon=True)
        reader.start()
        deadline, output = time.monotonic() + 300, []
        while True:
            if time.monotonic() >= deadline:
                raise ValueError("native Windows PKI acceptance deadline expired")
            driver.poll()
            try:
                line = lines.get(timeout=0.02)
            except queue.Empty:
                continue
            if line is None:
                break
            if sum(map(len, output)) + len(line) > 1024 * 1024:
                raise ValueError("native Windows PKI evidence exceeded bounds")
            output.append(line)
            print(line, end="", flush=True)
            match = re.search(r"EVIDENCE windows-pki case=([a-z_]+) stage=begin", line)
            if match:
                driver.case = match.group(1)
        if child.wait(timeout=10) != 0:
            raise ValueError("real native Windows PKI test failed")
        server.communicate("stop\n", timeout=10)
        if server.returncode != 0:
            raise ValueError("local native TLS server did not stop cleanly")
        report = json.loads((fixture.directory / "server-events.json").read_text())
        events = report["events"]
        expected = metadata["fingerprints_sha256"]["client"]
        alternate = metadata["fingerprints_sha256"]["alternate_client"]
        if not any(event["endpoint"] == "mtls" and event["status"] == 200 and event["client_fingerprint_sha256"] == expected for event in events):
            raise ValueError("server never authenticated the exact OS-held fixture key")
        if not any(event["endpoint"] == "mtls" and event["status"] == 403 and event["client_fingerprint_sha256"] == alternate for event in events):
            raise ValueError("server never rejected the selected alternate identity")
        if any(event["status"] > 0 and event["endpoint"] in ("redirect_mtls", "untrusted", "wrong_hostname", "expired") for event in events):
            raise ValueError("negative TLS/origin case unexpectedly received HTTP")
        if report["overflow"] or not driver.readonly_verified or not {"missing_identity", "exact_identity", "changed_origin", "cancelled_request"} <= driver.seen:
            raise ValueError("native review evidence incomplete")
        result = {"schema_version": 1, "fixture_id": metadata["fixture_id"], "native_browser_verified": True,
            "production_acceptance": False, "readonly_native_review_and_cancel_default": True, "events": events}
    finally:
        try:
            if child is not None and child.poll() is None:
                child.kill()
                child.wait(timeout=10)
        finally:
            try:
                if server is not None and server.poll() is None:
                    try:
                        server.communicate("stop\n", timeout=10)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait(timeout=10)
            finally:
                try:
                    fixture.close()
                finally:
                    try:
                        if owner is not None:
                            owner.close()
                    finally:
                        for path in fixture.directory.iterdir():
                            if path.suffix in (".key", ".pfx") or path.name == "passphrase.txt":
                                path.unlink()
        if result is not None:
            result["os_store_cleanup_verified"] = True
            private_write(fixture.directory / "windows-pki-acceptance.json", (json.dumps(result, indent=2)+"\n").encode())
            print("PASS actual Windows CEF CA/mTLS login, native consent and exact fixture store/key cleanup", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--state-parent", type=Path, required=True)
    parser.add_argument("--test-executable", type=Path, required=True)
    parser.add_argument("--server-executable", type=Path, required=True)
    parser.add_argument("--test", default="system_ca_and_reviewed_client_identity_authenticate_through_real_cef")
    parser.add_argument("--confirm-fixture-store-changes", action="store_true", required=True)
    args = parser.parse_args()
    for name in ("directory", "component", "state_parent", "test_executable", "server_executable"):
        setattr(args, name, getattr(args, name).absolute())
    run(args)


if __name__ == "__main__":
    main()
