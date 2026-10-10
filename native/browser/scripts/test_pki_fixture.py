"""Actual TLS/mTLS fixture conformance, independent from Chromium acceptance."""

import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import shutil
import socket
import ssl
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
from urllib.parse import urlsplit

import pki_fixture
from pki_fixture_proxy import FixtureProxy


@unittest.skipUnless(shutil.which("openssl"), "disposable PKI requires OpenSSL")
class PkiFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.root = Path(cls.temporary.name) / "pki"
        cls.fixture = pki_fixture.Fixture(cls.root)
        cls.metadata = cls.fixture.start()

    @classmethod
    def tearDownClass(cls):
        cls.fixture.close()
        cls.temporary.cleanup()

    def context(self, identity=None, trusted=True):
        context = ssl.create_default_context(cafile=self.root / "ca.pem" if trusted else None)
        if identity:
            context.load_cert_chain(self.root / f"{identity}.pem", self.root / f"{identity}.key")
        return context

    def request(self, endpoint, *, path="/public", identity=None, trusted=True):
        address = urlsplit(self.metadata["urls"][endpoint])
        connection = http.client.HTTPSConnection(address.hostname, address.port,
            timeout=3, context=self.context(identity, trusted))
        try:
            connection.request("GET", path)
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read(4096)
        finally:
            connection.close()

    def test_material_is_private_unique_and_no_secret_is_in_metadata(self):
        value = json.dumps(self.metadata)
        self.assertNotIn((self.root / "passphrase.txt").read_text("ascii"), value)
        self.assertNotIn("PRIVATE KEY", value)
        self.assertFalse(self.metadata["changes_os_trust"])
        self.assertFalse(self.metadata["production_acceptance"])
        self.assertEqual(len(set(self.metadata["fingerprints_sha256"].values())), 10)
        for name, expected in self.metadata["fingerprints_sha256"].items():
            self.assertEqual(hashlib.sha256((self.root / f"{name}.der").read_bytes()).hexdigest(),
                             expected)
        if os.name != "nt":
            self.assertEqual(stat.S_IMODE(self.root.stat().st_mode), 0o700)
            for path in self.root.iterdir():
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_generation_cannot_overwrite_an_existing_directory(self):
        before = (self.root / "ca.der").read_bytes()
        with self.assertRaises((ValueError, FileExistsError)):
            pki_fixture.Fixture(self.root)
        self.assertEqual((self.root / "ca.der").read_bytes(), before)

    def test_cli_control_reports_bounded_public_evidence_and_stops(self):
        with tempfile.TemporaryDirectory() as directory:
            allocation = Path(directory) / "private"
            result = subprocess.run([sys.executable, "-B", str(Path(pki_fixture.__file__)),
                "--directory", str(allocation)], input="report\nstop\n", text=True,
                capture_output=True, timeout=10, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = result.stdout.splitlines()
            self.assertEqual(len(lines), 2)
            ready, report = map(json.loads, lines)
            self.assertTrue(ready["ready"])
            self.assertEqual(ready["directory"], str(allocation))
            self.assertEqual(len(ready["fixture"]["urls"]), 6)
            self.assertEqual(report["report"]["fixture_id"], ready["fixture"]["fixture_id"])
            self.assertEqual(report["report"]["events"], [])
            self.assertFalse(report["report"]["native_browser_verified"])
            self.assertFalse(report["report"]["os_store_cleanup_verified"])
            self.assertNotIn((allocation / "passphrase.txt").read_text("ascii"), result.stdout)

    def test_private_ca_succeeds_without_changing_os_trust(self):
        status, _, body = self.request("public")
        self.assertEqual(status, 200)
        self.assertIn(b"private CA verified", body)
        with self.assertRaises((ssl.SSLError, OSError)):
            self.request("public", trusted=False)

    def test_unknown_ca_wrong_hostname_and_expired_server_are_denied(self):
        for endpoint in ("untrusted", "wrong_hostname", "expired"):
            with self.subTest(endpoint=endpoint), self.assertRaises((ssl.SSLError, OSError)):
                self.request(endpoint)
        forbidden = {"untrusted", "wrong_hostname", "expired"}
        self.assertFalse(any(event["endpoint"] in forbidden and event["status"] == 200
                             for event in self.fixture.report()["events"]))

    def test_mtls_records_actual_server_observed_certificate(self):
        status, _, body = self.request("mtls", path="/client-check", identity="client")
        self.assertEqual(status, 200)
        self.assertIn(b"identity verified", body)
        expected = self.metadata["fingerprints_sha256"]["client"]
        self.assertTrue(any(event["endpoint"] == "mtls" and event["status"] == 200
                            and event["client_fingerprint_sha256"] == expected
                            for event in self.fixture.report()["events"]))
        self.assertEqual(hashlib.sha256(ssl.PEM_cert_to_DER_cert(
            (self.root / "client.pem").read_text("ascii"))).hexdigest(), expected)

    def test_missing_wrong_issuer_expired_and_wrong_usage_client_are_denied(self):
        for identity in (None, "wrong_issuer_client", "expired_client", "wrong_usage_client"):
            with self.subTest(identity=identity), self.assertRaises((ssl.SSLError, OSError)):
                self.request("mtls", path="/client-check", identity=identity)

    def test_another_valid_identity_is_observed_and_rejected_by_exact_fingerprint(self):
        status, _, _ = self.request("mtls", path="/client-check", identity="alternate_client")
        self.assertEqual(status, 403)
        expected = self.metadata["fingerprints_sha256"]["alternate_client"]
        self.assertTrue(any(event["status"] == 403 and event["client_fingerprint_sha256"] == expected
                            for event in self.fixture.report()["events"]))

    def test_redirect_changes_origin_and_foreign_origin_requires_its_own_identity(self):
        status, headers, _ = self.request("public", path="/redirect-denied")
        self.assertEqual(status, 302)
        self.assertEqual(headers["Location"], self.metadata["urls"]["redirect_mtls"] + "/client-check")
        self.assertNotEqual(self.metadata["urls"]["mtls"], self.metadata["urls"]["redirect_mtls"])
        with self.assertRaises((ssl.SSLError, OSError)):
            self.request("redirect_mtls", path="/client-check")

    def test_redirect_can_start_after_the_server_observes_the_bound_identity(self):
        status, headers, _ = self.request("mtls", path="/redirect-denied", identity="client")
        self.assertEqual(status, 302)
        self.assertEqual(headers["Location"], self.metadata["urls"]["redirect_mtls"] + "/client-check")
        expected = self.metadata["fingerprints_sha256"]["client"]
        self.assertTrue(any(event["endpoint"] == "mtls" and event["status"] == 302
                            and event["client_fingerprint_sha256"] == expected
                            for event in self.fixture.report()["events"]))

    def test_events_are_bounded_and_never_claim_native_or_store_acceptance(self):
        # Event accounting has no dependency on certificate generation.
        fixture = object.__new__(pki_fixture.Fixture)
        fixture.metadata = {"fixture_id": "test"}
        fixture._lock = threading.Lock()
        fixture._events, fixture._overflow = [], False
        for _ in range(513):
            fixture.event("mtls", "/client-check", "a" * 64, 200)
        report = fixture.report()
        self.assertEqual(len(report["events"]), 512)
        self.assertTrue(report["overflow"])
        self.assertFalse(report["native_browser_verified"])
        self.assertFalse(report["os_store_cleanup_verified"])

    def test_excessive_http_headers_never_reach_a_fixture_route(self):
        address = urlsplit(self.metadata["urls"]["public"])
        with socket.create_connection((address.hostname, address.port), timeout=3) as raw:
            with self.context().wrap_socket(raw, server_hostname=address.hostname) as secured:
                secured.sendall(b"GET /public HTTP/1.1\r\n" + b"X-Bounded: " + b"x" * 17_000 + b"\r\n\r\n")
                try:
                    response = secured.recv(4096)
                except OSError:
                    response = b""
                self.assertNotIn(b"200 OK", response)

    def test_close_reaps_an_accepted_socket_waiting_for_request_headers(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = pki_fixture.Fixture(Path(directory) / "private")
            metadata = fixture.start()
            address = urlsplit(metadata["urls"]["public"])
            context = ssl.create_default_context(cafile=fixture.directory / "ca.pem")
            try:
                with socket.create_connection((address.hostname, address.port), timeout=3) as raw:
                    with context.wrap_socket(raw, server_hostname=address.hostname) as secured:
                        secured.sendall(b"GET /public HTTP/1.1\r\n")
                        fixture.close()
                        try:
                            response = secured.recv(4096)
                        except OSError:
                            response = b""
                        self.assertEqual(response, b"")
                        self.assertFalse(fixture._threads)
                        self.assertFalse(fixture._servers)
            finally:
                fixture.close()

    def test_authenticated_proxy_relays_only_an_exact_owned_tls_fixture(self):
        proxy = FixtureProxy(self.metadata)
        target = urlsplit(self.metadata["urls"]["mtls"])
        headers = {"Proxy-Authorization": "Basic " + base64.b64encode(
            ("colossus:" + proxy.password).encode()).decode()}
        connection = http.client.HTTPSConnection("127.0.0.1", proxy.server.server_address[1],
            context=self.context("client"), timeout=3)
        connection.set_tunnel(target.hostname, target.port, headers=headers)
        try:
            connection.request("GET", "/client-check")
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            response.read(4096)
            self.assertEqual(proxy.requests, [f"127.0.0.1:{target.port}"])
        finally:
            connection.close()
            proxy.close()

    def test_proxy_denies_missing_authentication_and_unowned_destination(self):
        proxy = FixtureProxy(self.metadata)
        try:
            for authority, header in (("127.0.0.1:1", ""), ("unowned.invalid:443", "")):
                with socket.create_connection(proxy.server.server_address, timeout=3) as connection:
                    connection.sendall(f"CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{header}\r\n".encode())
                    self.assertIn(b"407", connection.recv(4096))
            authorization = base64.b64encode(("colossus:" + proxy.password).encode()).decode()
            with socket.create_connection(proxy.server.server_address, timeout=3) as connection:
                connection.sendall(("CONNECT unowned.invalid:443 HTTP/1.1\r\nHost: unowned.invalid:443\r\n"
                    f"Proxy-Authorization: Basic {authorization}\r\n\r\n").encode())
                self.assertIn(b"403", connection.recv(4096))
            self.assertEqual(proxy.requests, [])
        finally:
            proxy.close()


if __name__ == "__main__":
    unittest.main()
