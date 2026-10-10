"""Hostile archive and installed-component integrity tests; no network required."""

import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from urllib.request import Request
from unittest.mock import patch

import component


class ComponentTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def archive(self, entries):
        path = self.root / "source.tar.bz2"
        with tarfile.open(path, "w:bz2") as archive:
            for name, kind, data in entries:
                member = tarfile.TarInfo(name)
                member.mode = 0o755 if kind == "directory" else 0o644
                if kind == "directory":
                    member.type = tarfile.DIRTYPE
                    archive.addfile(member)
                elif kind in ("symlink", "hardlink"):
                    member.type = tarfile.SYMTYPE if kind == "symlink" else tarfile.LNKTYPE
                    member.linkname = data
                    archive.addfile(member)
                elif kind == "fifo":
                    member.type = tarfile.FIFOTYPE
                    archive.addfile(member)
                else:
                    payload = data.encode()
                    member.size = len(payload)
                    archive.addfile(member, io.BytesIO(payload))
        return path

    def stage_component(self):
        root = self.root / "installed"
        root.mkdir()
        program = root / "colossus-browser"
        program.write_bytes(b"fixture executable")
        program.chmod(0o755)
        (root / "resources").mkdir()
        (root / "resources" / "icudtl.dat").write_bytes(b"icu fixture")
        component.installed_inventory(root, "linux64", "colossus-browser")
        return root

    def test_checked_in_pins_are_complete_and_fixed(self):
        lock = component.load_lock()
        self.assertEqual(set(lock["archives"]), set(component.PLATFORMS))
        self.assertEqual(lock["source_origin"], component.ORIGIN)

    @unittest.skipIf(os.name == "nt", "POSIX cache ownership/permissions")
    def test_other_user_writable_cache_is_refused_before_fetch(self):
        cache = self.root / "cache"
        cache.mkdir()
        cache.chmod(0o777)
        with patch.object(component, "fetch_archive") as fetch:
            with self.assertRaises(component.ComponentError):
                component.provision(cache, "linux64")
            fetch.assert_not_called()

    def test_archive_digest_and_length_are_required(self):
        archive = self.archive([("cef", "directory", ""), ("cef/file", "file", "payload")])
        pin = {"size": archive.stat().st_size, "sha256": component.digest_file(archive)}
        component.verify_archive(archive, pin)
        with self.assertRaises(component.ComponentError):
            component.verify_archive(archive, {**pin, "sha256": "0" * 64})
        with self.assertRaises(component.ComponentError):
            component.verify_archive(archive, {**pin, "size": pin["size"] + 1})

    def test_source_cache_inventory_comes_from_archive_bytes(self):
        archive = self.archive([
            ("cef", "directory", ""), ("cef/resources", "directory", ""),
            ("cef/resources/file", "file", "original"),
        ])
        destination = self.root / "source"
        component.extract_archive(archive, destination, "cef")
        original = component.source_archive_inventory(archive, "cef")
        self.assertTrue(component.source_inventory_matches(
            original, component.inventory_files(destination, component.SOURCE_MANIFEST)))
        (destination / "resources/file").write_text("changed")
        forged_receipt = component.inventory_files(destination, component.SOURCE_MANIFEST)
        self.assertFalse(component.source_inventory_matches(original, forged_receipt))

    def test_windows_source_comparison_ignores_only_unrepresentable_posix_modes(self):
        expected = [
            {"path": "Release", "kind": "directory", "mode": 0o755},
            {"path": "Release/libcef.dll", "kind": "file", "size": 42,
             "sha256": "a" * 64, "mode": 0o644},
            {"path": "alias", "kind": "symlink", "target": "Release",
             "sha256": "b" * 64},
        ]
        windows = [{**record, "mode": 0o777 if record["kind"] == "directory" else 0o666}
                   if "mode" in record else record.copy() for record in expected]
        with patch.object(component.os, "name", "nt"):
            self.assertTrue(component.source_inventory_matches(expected, windows))
            for field, changed in (("size", 43), ("sha256", "0" * 64),
                                   ("path", "Release/foreign.dll"), ("kind", "symlink"),
                                   ("mode", 0o444), ("mode", "writable")):
                tampered = [record.copy() for record in windows]
                tampered[1][field] = changed
                self.assertFalse(component.source_inventory_matches(expected, tampered))
            tampered = [record.copy() for record in windows]
            tampered[2]["target"] = "foreign"
            self.assertFalse(component.source_inventory_matches(expected, tampered))
            self.assertFalse(component.source_inventory_matches(expected, windows[:-1]))
            self.assertFalse(component.source_inventory_matches(expected, windows + [windows[-1]]))
            self.assertFalse(component.source_inventory_matches(expected, [None]))
            self.assertFalse(component.source_inventory_matches(expected, None))
        with patch.object(component.os, "name", "posix"):
            self.assertFalse(component.source_inventory_matches(expected, windows))

    def test_portable_path_rejections(self):
        for path in ("/root", "../escape", "cef/../escape", "cef/./file", "cef//file",
                     "C:/escape", "cef\\file", "cef/NUL", "cef/name.", "cef/name ", "cef/\x00x"):
            with self.subTest(path=path), self.assertRaises(component.ComponentError):
                component.safe_path(path)

    def test_archive_attack_entries_fail_before_writing(self):
        attacks = [
            [("cef/../../escape", "file", "attack")],
            [("/absolute", "file", "attack")],
            [("cef/link", "symlink", "../../escape")],
            [("cef/link", "symlink", "/absolute")],
            [("cef/file", "hardlink", "cef/existing")],
            [("cef/fifo", "fifo", "")],
            [("cef/File", "file", "one"), ("cef/file", "file", "two")],
            [("cef/file", "file", "one"), ("cef/file", "file", "two")],
            [("cef/link", "symlink", "missing")],
            [("cef/link", "symlink", "other"), ("cef/other", "symlink", "link")],
            [("cef/dir", "directory", ""), ("cef/link", "symlink", "dir"),
             ("cef/link/file", "file", "attack")],
            [("cef/file", "file", "payload"), ("cef/file/nested", "file", "attack")],
            [("different/file", "file", "attack")],
        ]
        for index, entries in enumerate(attacks):
            destination = self.root / f"extracted-{index}"
            with self.subTest(entries=entries), self.assertRaises(component.ComponentError):
                component.extract_archive(self.archive(entries), destination, "cef")
            self.assertFalse(destination.exists())
            self.assertFalse((self.root.parent / "escape").exists())

    def test_archive_entry_and_unpacked_size_limits(self):
        archive = self.archive([("cef", "directory", ""), ("cef/file", "file", "payload")])
        for constant, limit in (("MAX_FILES", 1), ("MAX_UNPACKED", 2)):
            with self.subTest(constant=constant), patch.object(component, constant, limit):
                with self.assertRaises(component.ComponentError):
                    component.extract_archive(archive, self.root / "limited", "cef")
                self.assertFalse((self.root / "limited").exists())

    def test_download_verifies_pin_and_cleans_failed_partial_archive(self):
        class Response(io.BytesIO):
            headers = {}

        cache = self.root / "cache"
        cache.mkdir()
        payload = b"download fixture"
        pin = {"filename": "fixture.tar.bz2", "size": len(payload),
               "sha256": hashlib.sha256(payload).hexdigest()}
        for override in ({"sha256": "0" * 64}, {"size": 3}):
            with self.subTest(override=override), patch.object(component, "build_opener") as opener:
                opener.return_value.open.return_value = Response(payload)
                with self.assertRaises(component.ComponentError):
                    component.fetch_archive(cache, "linux64", {**pin, **override})
                self.assertEqual(list(cache.iterdir()), [])
        with patch.object(component, "build_opener") as opener:
            opener.return_value.open.return_value = Response(payload)
            archive = component.fetch_archive(cache, "linux64", pin)
            self.assertEqual(archive.read_bytes(), payload)
            opener.return_value.open.assert_called_once_with(
                component.ORIGIN + "/fixture.tar.bz2", timeout=60,
            )

    @unittest.skipIf(os.name == "nt", "symlink creation needs Windows Developer Mode")
    def test_valid_framework_relative_links_are_preserved(self):
        archive = self.archive([
            ("cef", "directory", ""), ("cef/Framework", "directory", ""),
            ("cef/Framework/Versions", "directory", ""),
            ("cef/Framework/Versions/A", "directory", ""),
            ("cef/Framework/Versions/A/binary", "file", "framework"),
            ("cef/Framework/Versions/Current", "symlink", "A"),
            ("cef/Framework/binary", "symlink", "Versions/Current/binary"),
        ])
        destination = self.root / "extracted"
        component.extract_archive(archive, destination, "cef")
        self.assertEqual((destination / "Framework/binary").read_text(), "framework")
        self.assertTrue((destination / "Framework/Versions/Current").is_symlink())
        records = component.inventory_files(destination, component.MANIFEST)
        self.assertEqual(sum(record["kind"] == "symlink" for record in records), 2)

    def test_component_modes_remain_unvalidated(self):
        root = self.stage_component()
        manifest = component.verify_installed(root)
        self.assertEqual(manifest["protocol_version"], 1)
        self.assertEqual(manifest["modes"], {"desktop": False, "headless": False})
        manifest["modes"]["desktop"] = True
        (root / component.MANIFEST).write_text(json.dumps(manifest))
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)

    def test_modified_file_is_rejected(self):
        root = self.stage_component()
        (root / "resources/icudtl.dat").write_bytes(b"modified")
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)

    def test_added_file_and_directory_are_rejected(self):
        root = self.stage_component()
        addition = root / "resources/extra"
        addition.mkdir()
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)
        addition.rmdir()
        addition.write_bytes(b"added")
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)

    def test_missing_file_and_changed_mode_are_rejected(self):
        root = self.stage_component()
        program = root / "colossus-browser"
        program.chmod(0o644)
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)
        program.unlink()
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)

    @unittest.skipIf(os.name == "nt", "symlink creation needs Windows Developer Mode")
    def test_symlink_replacement_cannot_escape_component(self):
        root = self.stage_component()
        resource = root / "resources/icudtl.dat"
        resource.unlink()
        resource.symlink_to("../../outside")
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root)

    def test_hardlinked_files_are_refused(self):
        original = self.root / "file"
        original.write_bytes(b"payload")
        os.link(original, self.root / "alias")
        with self.assertRaises(component.ComponentError):
            component.digest_file(original)

    def test_manifest_cannot_replace_existing_inventory(self):
        root = self.stage_component()
        with self.assertRaises(component.ComponentError):
            component.installed_inventory(root, "linux64", "colossus-browser")

    def test_external_development_inventory_stays_outside_signed_component(self):
        root = self.stage_component()
        (root / component.MANIFEST).unlink()
        external = self.root / "external-browser-component.json"
        component.installed_inventory(root, "linux64", "colossus-browser", external)
        self.assertFalse((root / component.MANIFEST).exists())
        self.assertEqual(component.verify_installed(root, external)["modes"],
                         {"desktop": False, "headless": False})
        with self.assertRaises(component.ComponentError):
            component.installed_inventory(root, "linux64", "colossus-browser", external)
        with self.assertRaises(component.ComponentError):
            component.installed_inventory(root, "linux64", "colossus-browser", root / "nested.json")
        (root / "colossus-browser").write_bytes(b"changed executable")
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root, external)

    def test_external_inventory_rejects_added_embedded_receipt_name(self):
        root = self.stage_component()
        (root / component.MANIFEST).unlink()
        external = self.root / "external-browser-component.json"
        component.installed_inventory(root, "linux64", "colossus-browser", external)
        (root / component.MANIFEST).write_text("unexpected file in signed component")
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root, external)

    @unittest.skipIf(os.name == "nt", "symlink creation needs Windows Developer Mode")
    def test_external_manifest_parent_alias_cannot_place_receipt_inside_component(self):
        root = self.stage_component()
        alias = self.root / "root-alias"
        alias.symlink_to(root, target_is_directory=True)
        manifest = alias / "new-receipt.json"
        with self.assertRaises(component.ComponentError):
            component.installed_inventory(root, "linux64", "colossus-browser", manifest)
        with self.assertRaises(component.ComponentError):
            component.verify_installed(root, alias / component.MANIFEST)
        self.assertFalse(manifest.exists())

    def test_executable_cannot_escape_or_be_missing(self):
        root = self.root / "empty"
        root.mkdir()
        for executable in ("../other", "missing"):
            with self.subTest(executable=executable), self.assertRaises(component.ComponentError):
                component.installed_inventory(root, "linux64", executable)

    def test_duplicate_json_keys_are_rejected(self):
        path = self.root / "manifest.json"
        path.write_text('{"key":1,"key":2}')
        with self.assertRaises(component.ComponentError):
            component.load_json(path)

    def test_redirects_remain_on_exact_archive_url(self):
        redirect = component.FixedOriginRedirects()
        request = Request(component.ORIGIN + "/archive.tar.bz2")
        for destination in (
            "http://cef-builds.spotifycdn.com/archive.tar.bz2",
            "https://attacker.test/archive.tar.bz2",
            component.ORIGIN + "/other.tar.bz2",
            component.ORIGIN + "/archive.tar.bz2?other",
            "https://name:password@cef-builds.spotifycdn.com/archive.tar.bz2",
        ):
            with self.subTest(destination=destination), self.assertRaises(component.ComponentError):
                redirect.redirect_request(request, None, 302, "Found", {}, destination)
        for _ in range(component.MAX_REDIRECTS):
            request = redirect.redirect_request(request, None, 302, "Found", {}, request.full_url)
        with self.assertRaises(component.ComponentError):
            redirect.redirect_request(request, None, 302, "Found", {}, request.full_url)


if __name__ == "__main__":
    unittest.main()
