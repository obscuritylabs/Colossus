#!/usr/bin/env python3
"""Explicit local CEF PKI login acceptance with new Windows fixture keys only.

Requires --confirm-fixture-store-changes. New CurrentUser ROOT/MY entries and
nonexportable CNG keys are recorded before use and removed in finally. The owned
Rust test uses the authenticated native review channel and exact generated
fingerprints. This runner never operates browser certificate-picker windows;
the fixture store helper acknowledges only its own generated-root confirmation.
"""
from __future__ import annotations
import argparse
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


CASES = ("unique_initial_identity", "trusted_ca", "missing_identity", "alternate_identity",
         "exact_identity", "changed_origin", "untrusted_root", "wrong_hostname",
         "expired_server", "cancelled_request")
DEFAULT_TEST = "system_ca_and_reviewed_client_identity_authenticate_through_real_cef"


class ReviewEvidence:
    """Finite test-owned protocol; provision a second key only after unique login."""
    def __init__(self, test_name=DEFAULT_TEST):
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_:]{0,255}", test_name):
            raise ValueError("one literal bounded Rust test name required")
        self.prelude, self.prelude_seen = f"test {test_name} ... ", False
        self.begun, self.passed, self.alternate_requested = [], [], False

    def observe(self, line):
        if line.startswith("test ") and " ... " in line:
            if (self.prelude_seen or self.begun or self.passed or self.alternate_requested
                    or not line.startswith(self.prelude)):
                raise ValueError("foreign or repeated Rust test prelude")
            line = line[len(self.prelude):]
            if line.rstrip("\r\n") != f"EVIDENCE windows-pki case={CASES[0]} stage=begin":
                raise ValueError("Rust test prelude must prefix only the first PKI record")
            self.prelude_seen = True
        if line.rstrip("\r\n") == "EVIDENCE windows-pki stage=provision_alternate":
            if self.alternate_requested or self.passed != [CASES[0]]:
                raise ValueError("alternate identity provisioning was not requested once after unique login")
            self.alternate_requested = True
            return True
        match = re.fullmatch(r"EVIDENCE windows-pki case=([a-z_]+) stage=begin\s*", line)
        if match:
            name = match.group(1)
            if (len(self.begun) >= len(CASES) or name != CASES[len(self.begun)]
                    or self.begun != self.passed or (self.begun and not self.alternate_requested)):
                raise ValueError("native PKI case ordering was not exact")
            self.begun.append(name)
        match = re.fullmatch(r"PASS windows-pki case=([a-z_]+)(?: core_effect_gateway=true)?\s*", line)
        if match:
            name = match.group(1)
            if len(self.begun) != len(self.passed) + 1 or self.begun[-1] != name:
                raise ValueError("native PKI completion was not correlated")
            self.passed.append(name)
        return False

    def complete(self):
        return self.alternate_requested and self.begun == self.passed == list(CASES)


def provision(fixture, owner, *, labels=("client", "alternate_client"), include_root=True):
    if not labels or len(set(labels)) != len(labels) or any(label not in ("client", "alternate_client") for label in labels):
        raise ValueError("finite fixture identity labels required")
    directory, metadata = fixture.directory, fixture.metadata
    ca = x509.load_pem_x509_certificate((directory / "ca.pem").read_bytes())
    ca_key = serialization.load_pem_private_key((directory / "ca.key").read_bytes(), None)
    now = datetime.now(timezone.utc)
    for label in labels:
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
    if include_root:
        owner.add_certificate("ROOT", ca.public_bytes(serialization.Encoding.DER))


def run(arguments):
    fixture = Fixture(arguments.directory)
    owner = None
    result = None
    child = None
    server = None
    try:
        owner = FixtureStore(fixture.directory, fixture.metadata["fixture_id"], confirm_generated_roots=True)
        fixture.metadata["fingerprints_sha256"].pop("alternate_client", None)
        provision(fixture, owner, labels=("client",))
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
        evidence = ReviewEvidence(arguments.test)
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
            if evidence.observe(line):
                provision(fixture, owner, labels=("alternate_client",), include_root=False)
                private_write(fixture.directory / "alternate-client-ready.json", (json.dumps(metadata)+"\n").encode())
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
        if report["overflow"] or not evidence.complete():
            raise ValueError("authenticated native review evidence incomplete")
        result = {"schema_version": 1, "fixture_id": metadata["fixture_id"], "native_browser_verified": True,
            "production_acceptance": False, "authenticated_native_review_channel": True,
            "unique_initial_identity_without_view_lease": True, "standalone_dialog_automation": False, "events": events}
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
            print("PASS actual Windows CEF CA/mTLS login, authenticated native review and exact fixture store/key cleanup", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--state-parent", type=Path, required=True)
    parser.add_argument("--test-executable", type=Path, required=True)
    parser.add_argument("--server-executable", type=Path, required=True)
    parser.add_argument("--test", default=DEFAULT_TEST)
    parser.add_argument("--confirm-fixture-store-changes", action="store_true", required=True)
    args = parser.parse_args()
    for name in ("directory", "component", "state_parent", "test_executable", "server_executable"):
        setattr(args, name, getattr(args, name).absolute())
    run(args)


if __name__ == "__main__":
    main()
