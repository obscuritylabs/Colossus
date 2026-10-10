"""Installer ownership and artifact-integrity tests using harmless synthetic bytes."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stage_nss_tools


class StageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="colossus-nss-stage-test-")
        self.root = Path(self.temporary.name)
        self.source = self.root / "synthetic-tool"
        self.source.write_bytes(b"synthetic-public-tool")
        self.source.chmod(0o555)
        self.lock = self.root / "lock.json"
        self.lock.write_text(json.dumps({"platform": "fixture", "nss_package_version": "fixture",
            "nspr_package_version": "fixture", "artifacts": {"certutil": {
                "source": str(self.source), "sha256": hashlib.sha256(self.source.read_bytes()).hexdigest()}}}))
        self.patch = patch.object(stage_nss_tools, "LOCK", self.lock)
        self.patch.start()

    def tearDown(self):
        self.patch.stop()
        for path in self.root.iterdir():
            if path.is_dir() and not path.is_symlink():
                path.chmod(0o700)
        self.temporary.cleanup()

    def test_staged_artifact_is_independent_verified_and_no_clobber(self):
        destination = self.root / "stage"
        receipt = stage_nss_tools.stage(destination)
        staged = destination / "certutil"
        self.assertEqual(staged.read_bytes(), self.source.read_bytes())
        self.assertNotEqual(staged.stat().st_ino, self.source.stat().st_ino)
        self.assertEqual(hashlib.sha256(staged.read_bytes()).hexdigest(), receipt["artifacts"]["certutil"])
        self.assertFalse(receipt["production_acceptance"])
        with self.assertRaises(ValueError):
            stage_nss_tools.stage(destination)

    def test_changed_artifact_relative_destination_and_symlink_are_rejected(self):
        with self.assertRaises(ValueError):
            stage_nss_tools.stage(Path("relative-stage"))
        self.source.chmod(0o755)
        self.source.write_bytes(b"changed")
        with self.assertRaises(ValueError):
            stage_nss_tools.stage(self.root / "changed")
        self.assertFalse((self.root / "changed").exists())
        linked = self.root / "linked"
        linked.symlink_to(self.source)
        lock = json.loads(self.lock.read_text())
        lock["artifacts"]["certutil"]["source"] = str(linked)
        self.lock.write_text(json.dumps(lock))
        with self.assertRaises(ValueError):
            stage_nss_tools.stage(self.root / "linked-stage")

    def test_copy_race_fails_and_removes_only_new_stage(self):
        original = stage_nss_tools.shutil.copyfile
        def tamper(source, target):
            original(source, target)
            target.write_bytes(b"tampered-during-copy")
        with patch.object(stage_nss_tools.shutil, "copyfile", tamper):
            with self.assertRaises(ValueError):
                stage_nss_tools.stage(self.root / "raced")
        self.assertFalse((self.root / "raced").exists())
        self.assertTrue(self.source.exists())


if __name__ == "__main__":
    unittest.main()
