"""Pure finite-source/result/plain-marker contracts; no native launch/signature API."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import prepare_custody_front_probe as probe

class SourceContract(unittest.TestCase):
    def prepared(self):
        return {"native_source_sha256": {key: probe.digest(probe.ROOT / key) for key in probe.SOURCE_KEYS}, "controller_source_sha256": probe.digest(probe.ROOT / probe.CONTROLLER_SOURCE)}
    def test_exact_source_and_controller_proof_required(self):
        valid = self.prepared()
        probe.verify_source_receipt(valid)
        for change in ("missing", "extra", "escape", "controller", "changed"):
            value = copy.deepcopy(valid)
            if change == "missing": value["native_source_sha256"].pop(next(iter(probe.SOURCE_KEYS)))
            if change == "extra": value["native_source_sha256"]["native/browser/unreviewed.c"] = "a" * 64
            if change == "escape": value["native_source_sha256"]["../../private"] = "a" * 64
            if change == "controller": value["controller_source_sha256"] = "a" * 64
            if change == "changed": value["native_source_sha256"][next(iter(probe.SOURCE_KEYS))] = "a" * 64
            with self.subTest(change=change), self.assertRaises(ValueError): probe.verify_source_receipt(value)
    def test_receipt_role_bits_types_and_unknown_fields_fail_closed(self):
        valid = {"schema_version":1,"role":2,"pre_body_denials":511,"direct_child_reaped":True,"exit_code":0,"cef_loaded":False,"production_accepted":False}
        self.assertTrue(probe.accepted_receipt(valid, 2))
        for key, replacement in (("role",3),("pre_body_denials",255),("direct_child_reaped",1),("cef_loaded",True),("production_accepted",True),("exit_code",1)):
            value = dict(valid);value[key] = replacement
            self.assertFalse(probe.accepted_receipt(value, 2))
        self.assertFalse(probe.accepted_receipt(dict(valid, unreviewed=True), 2))
        self.assertFalse(probe.accepted_receipt(None, 2))

@unittest.skipUnless(os.name == "posix", "POSIX ownership/private-mode and no-follow markers run on Linux/macOS CI")
class MarkerContract(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        (self.root / "custody").mkdir(mode=0o700)
        (self.root / "allocation/profile").mkdir(parents=True, mode=0o700)
        for name in probe.MARKERS:
            path = self.root / "custody" / name
            path.write_bytes(b"colossus public custody probe v1\n")
            path.chmod(0o600)
    def tearDown(self): self.temporary.cleanup()
    def test_all_five_positive_marker_generations_are_captured(self):
        first = probe.marker_generations(self.root)
        self.assertEqual(len(first), 5)
        self.assertEqual(first, probe.marker_generations(self.root))
    def test_each_missing_marker_is_rejected(self):
        for name in probe.MARKERS:
            path = self.root / "custody" / name
            saved = path.read_bytes();path.unlink()
            with self.subTest(name=name), self.assertRaises(OSError): probe.marker_generations(self.root)
            path.write_bytes(saved);path.chmod(0o600)
    def test_changed_contents_link_and_permissions_are_rejected(self):
        path = self.root / "custody" / probe.MARKERS[-1]
        original = path.read_bytes();path.write_bytes(b"changed")
        with self.assertRaises(ValueError): probe.marker_generations(self.root)
        path.write_bytes(original);path.chmod(0o644)
        with self.assertRaises(ValueError): probe.marker_generations(self.root)
        path.chmod(0o600);os.link(path, self.root / "alias")
        with self.assertRaises(ValueError): probe.marker_generations(self.root)
    def test_both_creation_destinations_must_be_absent(self):
        for path in (self.root / "custody/.colossus-custody-probe-create", self.root / "allocation/profile/.colossus-custody-probe-alias"):
            path.touch()
            with self.assertRaises(ValueError): probe.marker_generations(self.root)
            path.unlink()
    def test_symlink_marker_never_substitutes_for_owned_file(self):
        path = self.root / "custody" / probe.MARKERS[1]
        other = self.root / "foreign";path.rename(other);path.symlink_to(other)
        with self.assertRaises(OSError): probe.marker_generations(self.root)

if __name__ == "__main__": unittest.main()
