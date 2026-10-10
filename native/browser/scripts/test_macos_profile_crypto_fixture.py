"""File-only runner guards; no native, Security, browser or audit execution."""

from pathlib import Path
import json
import os
import tempfile
import unittest
from unittest import mock

import macos_profile_crypto_fixture as fixture
from pki_fixture_private import private_directory, private_write


@unittest.skipIf(os.name == "nt", "POSIX private macOS fixture guards")
class FixtureGuards(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve(strict=True)
        self.artifacts = self.root / "artifacts"

    def artifact(self):
        private_directory(self.artifacts)
        private_directory(self.artifacts / "owned-state")
        for name in ("store-proof.stdout.json", "store-proof.stderr.log"):
            private_write(self.artifacts / name, b"")
        return self.artifacts

    def test_existing_artifacts_are_preserved_before_compilation(self):
        native = self.root / "native"
        native.mkdir()
        (native / "profile-crypto-development-build.json").write_text(json.dumps({
            "schema_version": 1, "development_only": True, "build_type": "Debug",
            "production_accepted": False, "api": "unsupported-dyld-security-spi",
        }))
        self.artifact()
        sentinel = self.artifacts / "keep"
        sentinel.write_bytes(b"private evidence")
        with mock.patch.object(fixture.subprocess, "run") as process:
            with self.assertRaises(fixture.FixtureError):
                fixture.build(native, self.artifacts)
            process.assert_not_called()
        self.assertEqual(sentinel.read_bytes(), b"private evidence")

    def test_nonempty_receipt_prevents_any_execution(self):
        self.artifact()
        receipt = self.artifacts / "store-proof.stdout.json"
        receipt.write_bytes(b"existing receipt")
        with mock.patch.object(fixture.subprocess, "run") as process:
            with self.assertRaises(fixture.FixtureError):
                fixture.run(self.artifacts)
            process.assert_not_called()
        self.assertEqual(receipt.read_bytes(), b"existing receipt")

    def test_unknown_owned_state_prevents_any_execution(self):
        self.artifact()
        state = self.artifacts / "owned-state/unknown"
        state.write_bytes(b"preserved")
        with mock.patch.object(fixture.subprocess, "run") as process:
            with self.assertRaises(fixture.FixtureError):
                fixture.run(self.artifacts)
            process.assert_not_called()
        self.assertEqual(state.read_bytes(), b"preserved")

    def test_interrupted_attempt_cannot_be_replayed(self):
        self.artifact()
        started = self.artifacts / "execution-started.json"
        private_write(started, b"{}")
        with mock.patch.object(fixture.subprocess, "run") as process:
            with self.assertRaises(fixture.FixtureError):
                fixture.run(self.artifacts)
            process.assert_not_called()
        self.assertEqual(started.read_bytes(), b"{}")

    def test_existing_result_without_start_marker_prevents_execution(self):
        self.artifact()
        private_write(self.artifacts / "execution-result.json", b"{}")
        with mock.patch.object(fixture.subprocess, "run") as process:
            with self.assertRaises(fixture.FixtureError):
                fixture.run(self.artifacts)
            process.assert_not_called()

    def manifest(self):
        names = ["reviewed-fixture.mm", "profile_crypto.mm", "profile_crypto.h",
                 "Scoped Profile Crypto Fixture.app/Contents/MacOS/scoped-profile-crypto-fixture",
                 "Scoped Profile Crypto Fixture.app/Contents/Frameworks/libcolossus_mac_profile_crypto.dylib"]
        for name in names:
            path = self.artifacts / name
            path.parent.mkdir(parents=True, exist_ok=True)
            private_write(path, b"original")
        private_write(self.artifacts / "build-evidence.json", json.dumps({
            "schema_version": 1, "executed_fixture": False, "production_acceptance": False,
            "certificate_pki_acceptance": False,
            "sha256": {name: fixture.digest(self.artifacts / name) for name in names},
        }).encode())
        return names

    def test_changed_reviewed_source_prevents_signature_or_execution(self):
        self.artifact()
        names = self.manifest()
        (self.artifacts / names[0]).write_bytes(b"changed source")
        with mock.patch.object(fixture.subprocess, "run") as process:
            with self.assertRaises(fixture.FixtureError):
                fixture.run(self.artifacts)
            process.assert_not_called()
        self.assertFalse((self.artifacts / "execution-started.json").exists())

    def test_failed_launch_marker_flush_prevents_native_execution(self):
        self.artifact()
        self.manifest()
        with mock.patch.object(fixture, "checked"), mock.patch.object(fixture.os, "fsync", side_effect=OSError("flush failed")):
            with mock.patch.object(fixture.subprocess, "run") as process:
                with self.assertRaises(OSError):
                    fixture.run(self.artifacts)
                process.assert_not_called()
        self.assertTrue((self.artifacts / "execution-started.json").exists())


if __name__ == "__main__":
    unittest.main()
