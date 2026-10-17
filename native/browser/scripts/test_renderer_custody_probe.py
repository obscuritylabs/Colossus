"""Boundary checks for test-only fixed native receipts; no simulated TLS evidence."""
import io
import http.client
import os
from pathlib import Path
import shutil
import ssl
import tempfile
import time
import unittest
from urllib.parse import urlsplit

from pki_fixture import Fixture
from run_host_pki_probe import NativeOutput


class Process:
    def __init__(self, stderr, stdout=b""):
        self.stdout = io.BytesIO(stdout)
        self.stderr = io.BytesIO(stderr)
        self.returncode = 0

    def wait(self, timeout):
        return self.returncode


class LivePipes(Process):
    """Actual buffered pipes whose writers deliberately stay open."""

    def __init__(self):
        stdout, stderr = os.pipe(), os.pipe()
        self.stdout, self.stderr = os.fdopen(stdout[0], "rb"), os.fdopen(stderr[0], "rb")
        self.writers = [stdout[1], stderr[1]]
        self.returncode = 0

    def close_writers(self):
        for descriptor in self.writers:
            os.close(descriptor)


class NativeReceiptTests(unittest.TestCase):
    def output(self, data, *, opted_in=False):
        result = NativeOutput(Process(data), custody_test=opted_in)
        result.join_output()
        return result

    def test_ordinary_pki_output_cannot_grant_renderer_custody_evidence(self):
        result = self.output(b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n")
        self.assertIsNone(result.custody_open_denied)

    def test_opted_in_receipts_require_exact_fixed_lines_and_failure_is_sticky(self):
        result = self.output(b"path secret COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n", opted_in=True)
        self.assertIsNone(result.custody_open_denied)
        result = self.output(b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n", opted_in=True)
        self.assertIs(result.custody_open_denied, True)
        result = self.output(b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_FAILED\nCOLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n", opted_in=True)
        self.assertIs(result.custody_open_denied, False)

    def test_loader_diagnostic_retains_only_fixed_category(self):
        private_path = b"/private-path-containing-secret-material/" + b"x" * 4090
        result = self.output(private_path + b": symbol lookup error: undefined symbol: close\n")
        self.assertEqual(result.startup_diagnostic, "elf_close_symbol_missing")
        self.assertIsNone(result.category)
        self.assertIsNone(result.custody_open_denied)
        self.assertFalse(any(isinstance(value, bytes) for value in vars(result).values()))

    def test_unknown_native_output_cannot_classify_or_grant_custody(self):
        result = self.output(b"arbitrary private diagnostic\n" + b"x" * 8192)
        self.assertIsNone(result.startup_diagnostic)
        self.assertIsNone(result.category)
        self.assertIsNone(result.custody_open_denied)

    def test_closed_native_phase_diagnostics_are_opt_in_exact_and_bounded(self):
        phase = b"COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_EXE_UNVERIFIED\n"
        self.assertEqual(self.output(phase).custody_diagnostics, [])
        result = self.output(b"private path " + phase + b"COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_UNKNOWN\n" +
                             phase * 20, opted_in=True)
        self.assertEqual(result.custody_diagnostics, ["exe_unverified"] * 16)
        self.assertIsNone(result.custody_open_denied)

    def test_combined_stream_failures_are_sticky_and_diagnostics_bounded(self):
        phase = b"COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_EXE_UNVERIFIED\n"
        result = NativeOutput(Process(phase * 100 + b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_FAILED\n",
                                      phase * 100 + b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n"), custody_test=True)
        result.join_output()
        self.assertIs(result.custody_open_denied, False)
        self.assertEqual(len(result.custody_diagnostics), 16)

    def test_fixed_receipt_is_decoded_from_a_live_buffered_pipe_before_eof(self):
        process = LivePipes()
        result = NativeOutput(process, custody_test=True)
        try:
            os.write(process.writers[1], b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n")
            self.assertIs(result.wait_custody_receipt(0.5), True)
            self.assertTrue(all(thread.is_alive() for thread in result.threads))
        finally:
            process.close_writers()
            result.join_output()

    def test_unmatched_live_line_times_out_and_late_receipt_does_not_change_wait_result(self):
        process = LivePipes()
        result = NativeOutput(process, custody_test=True)
        try:
            os.write(process.writers[0], b"private prefix COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n")
            started = time.monotonic()
            live_receipt = result.wait_custody_receipt(0.05)
            self.assertIsNone(live_receipt)
            self.assertGreaterEqual(time.monotonic() - started, 0.04)
            self.assertLess(time.monotonic() - started, 1)
            os.write(process.writers[1], b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED\n")
            self.assertIs(result.wait_custody_receipt(0.5), True)
            self.assertIsNone(live_receipt)
        finally:
            process.close_writers()
            result.join_output()

    def test_receipt_wait_is_opt_in_and_never_exceeds_the_closed_deadline(self):
        result = self.output(b"")
        self.assertIsNone(result.wait_custody_receipt())
        for timeout in (-1, 5.01, float("inf"), float("nan")):
            with self.subTest(timeout=timeout), self.assertRaises(ValueError):
                result.wait_custody_receipt(timeout)


@unittest.skipUnless(shutil.which("openssl"), "disposable PKI requires OpenSSL")
class RendererContextFixtureTests(unittest.TestCase):
    def test_literal_context_script_is_only_in_opted_in_exact_successful_mtls_response(self):
        with tempfile.TemporaryDirectory() as allocation:
            for enabled in (False, True):
                fixture = Fixture(Path(allocation) / str(enabled), renderer_context=enabled)
                try:
                    metadata = fixture.start()
                    for endpoint, path, identity in (
                            ("mtls", "/client-check", "client"),
                            ("mtls", "/client-check", "alternate_client"),
                            ("public", "/public", None),
                            ("redirect_mtls", "/client-check", "client")):
                        context = ssl.create_default_context(cafile=fixture.directory / "ca.pem")
                        if identity:
                            context.load_cert_chain(fixture.directory / f"{identity}.pem",
                                                    fixture.directory / f"{identity}.key")
                        target = urlsplit(metadata["urls"][endpoint])
                        connection = http.client.HTTPSConnection(target.hostname, target.port,
                            timeout=3, context=context)
                        try:
                            connection.request("GET", path)
                            response = connection.getresponse()
                            body = response.read(4096)
                            scripted = enabled and endpoint == "mtls" and identity == "client"
                            self.assertEqual(b"<script>void 0;</script>" in body, scripted)
                            if scripted:
                                self.assertEqual(response.status, 200)
                                self.assertIn(b"Colossus mTLS identity verified", body)
                        finally:
                            connection.close()
                    self.assertTrue(any(event["endpoint"] == "mtls" and event["status"] == 200
                        and event["client_fingerprint_sha256"] == metadata["fingerprints_sha256"]["client"]
                        for event in fixture.report()["events"]))
                finally:
                    fixture.close()


if __name__ == "__main__":
    unittest.main()
