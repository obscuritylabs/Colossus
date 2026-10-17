"""Real byte custody checks; these fixtures never grant publisher acceptance."""
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest

import component

from stage_cli_browser_bundle import BundleError, copy_verified, run_cli
from stage_macos import rename_exclusive


@unittest.skipUnless(os.name == "posix", "Linux offline package custody")
class OfflineBrowserByteCustody(unittest.TestCase):
    def accepted_inventory(self, root):
        root.mkdir()
        executable = root / "synthetic-browser"
        executable.write_bytes(b"synthetic inventory fixture")
        executable.chmod(0o500)
        manifest = component.installed_inventory(root, "linux64", executable.name)
        document = component.load_json(manifest)
        document["modes"] = {"desktop": False, "headless": True}
        manifest.write_text(json.dumps(document))
        return {"publisher_acceptance_verified": True, "payload_verified": True,
                "component_manifest_sha256": component.digest_file(manifest)}

    def test_accepted_inventory_has_a_separate_native_proof_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "component"
            proof = self.accepted_inventory(root)
            with self.assertRaises(component.ComponentError):
                component.verify_installed(root)
            inventory = component.verify_publisher_accepted_inventory(root, proof)
            self.assertEqual(inventory["modes"], {"desktop": False, "headless": True})

    def test_accepted_inventory_rechecks_proof_identity_and_component_bytes(self):
        for mutation in ("acceptance", "payload", "binding", "bytes"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "component"
                proof = self.accepted_inventory(root)
                if mutation == "acceptance":
                    proof["publisher_acceptance_verified"] = False
                elif mutation == "payload":
                    proof["payload_verified"] = False
                elif mutation == "binding":
                    proof["component_manifest_sha256"] = "0" * 64
                else:
                    (root / "synthetic-browser").chmod(0o700)
                    (root / "synthetic-browser").write_bytes(b"changed fixture")
                with self.assertRaises(component.ComponentError):
                    component.verify_publisher_accepted_inventory(root, proof)

    def test_copy_matches_exact_bytes_mode_size_and_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / "source", root / "target"
            source.write_bytes(b"public synthetic payload")
            copy_verified(source, target, hashlib.sha256(source.read_bytes()).hexdigest(), source.stat().st_size, 0o500, 128)
            self.assertEqual(target.read_bytes(), source.read_bytes())
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o500)

    def test_mismatched_digest_size_or_unsafe_source_cannot_leave_a_payload(self):
        for mutation in ("digest", "size", "world-write", "hardlink", "symlink"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source, target = root / "source", root / "target"
                source.write_bytes(b"bounded payload")
                digest, size = hashlib.sha256(source.read_bytes()).hexdigest(), source.stat().st_size
                if mutation == "digest":
                    digest = "0" * 64
                elif mutation == "size":
                    size += 1
                elif mutation == "world-write":
                    source.chmod(0o666)
                elif mutation == "hardlink":
                    os.link(source, root / "other")
                elif mutation == "symlink":
                    source.rename(root / "actual")
                    source.symlink_to("actual")
                with self.assertRaises((BundleError, OSError)):
                    copy_verified(source, target, digest, size, 0o600, 128)
                self.assertFalse(target.exists())

    def test_existing_destination_and_overlong_inputs_are_not_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / "source", root / "target"
            source.write_bytes(b"new")
            target.write_bytes(b"preserved")
            with self.assertRaises(FileExistsError):
                copy_verified(source, target, hashlib.sha256(b"new").hexdigest(), 3, 0o600, 128)
            self.assertEqual(target.read_bytes(), b"preserved")
            target.unlink()
            with self.assertRaises(BundleError):
                copy_verified(source, target, hashlib.sha256(b"new").hexdigest(), 3, 0o600, 2)
            self.assertFalse(target.exists())

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux exclusive package publication")
    def test_raced_package_destination_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / "stage", root / "published"
            source.mkdir()
            target.mkdir()
            (source / "owned").write_bytes(b"stage")
            with self.assertRaises(FileExistsError):
                rename_exclusive(source, target)
            self.assertTrue((source / "owned").exists())
            self.assertEqual(list(target.iterdir()), [])

    def test_cli_verification_is_bounded_and_reaps_its_owned_process(self):
        with tempfile.TemporaryDirectory() as directory:
            cli = Path(directory) / "fixture-cli"
            cli.write_text(f"#!{sys.executable}\nimport os\nos.write(1, b'{{\"schema_version\":1}}')\n")
            cli.chmod(0o700)
            self.assertEqual(run_cli(cli), {"schema_version": 1})
            cli.write_text(f"#!{sys.executable}\nimport os,time\nos.write(1, b'x' * 20000)\ntime.sleep(60)\n")
            with self.assertRaisesRegex(BundleError, "output bounds"):
                run_cli(cli)


if __name__ == "__main__":
    unittest.main()
