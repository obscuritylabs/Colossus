"""Development staging layout and failure contracts; no CEF or macOS required."""

import errno
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import component
import stage_macos


@unittest.skipIf(os.name == "nt", "CEF macOS staging requires POSIX links")
class MacosStageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.source = self.root / "sources"
        self.source.mkdir()
        framework = self.source / "Release" / stage_macos.FRAMEWORK
        (framework / "Libraries").mkdir(parents=True)
        (framework / "Resources/en.lproj").mkdir(parents=True)
        (framework / "Chromium Embedded Framework").write_bytes(b"framework fixture")
        for resource in ("icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak"):
            (framework / "Resources" / resource).write_text(resource)
        (framework / "Resources/en.lproj/locale.pak").write_text("locale fixture")
        for library in ("libcef_sandbox.dylib", "libEGL.dylib", "libGLESv2.dylib"):
            (framework / "Libraries" / library).write_text(library)
        for notice in ("LICENSE.txt", "CREDITS.html"):
            (self.source / notice).write_text(notice)
        self.build = self.root / "build"
        for suffix, _ in stage_macos.HELPERS:
            name = stage_macos.HELPER + suffix
            contents = self.build / "helpers" / (name + ".app") / "Contents"
            (contents / "MacOS").mkdir(parents=True)
            binary = contents / "MacOS" / name
            binary.write_bytes(b"helper fixture")
            binary.chmod(0o755)
            with (contents / "Info.plist").open("wb") as output:
                plistlib.dump({"CFBundleExecutable": name, "CFBundlePackageType": "APPL"}, output)
        self.executable = self.root / "browser-acceptance"
        self.executable.write_bytes(b"main fixture")
        self.executable.chmod(0o755)
        self.dictation = self.root / "dictation-assets"
        self.dictation.mkdir()
        for name in stage_macos.DICTATION_RESOURCES:
            (self.dictation / name).write_text(name)
        (self.dictation / "unrelated-file").write_text("not a bundled resource")
        self.app = self.root / "Colossus.app"
        self.verifier = patch.object(stage_macos, "verify_source", return_value=self.source)
        self.verifier.start()
        self.addCleanup(self.verifier.stop)

    def tearDown(self):
        self.temporary.cleanup()

    def stage(self):
        return stage_macos.stage_app(self.source, self.build, self.executable, self.app,
                                     "macosarm64", "com.colossus.acceptance", self.dictation)

    def test_complete_loader_layout_and_distinct_helper_identities(self):
        with patch.object(stage_macos, "run") as run:
            binary = self.stage()
        self.assertEqual(binary, self.app / "Contents/MacOS/browser-acceptance")
        frameworks = self.app / "Contents/Frameworks"
        framework = frameworks / stage_macos.FRAMEWORK
        self.assertTrue((framework / "Versions/Current").is_symlink())
        self.assertEqual((framework / "Libraries/libcef_sandbox.dylib").read_text(), "libcef_sandbox.dylib")
        self.assertEqual((framework / "Resources/en.lproj/locale.pak").read_text(), "locale fixture")
        identifiers = set()
        for suffix, identifier_suffix in stage_macos.HELPERS:
            name = stage_macos.HELPER + suffix
            helper = frameworks / (name + ".app")
            with (helper / "Contents/Info.plist").open("rb") as source:
                info = plistlib.load(source)
            self.assertEqual(info["CFBundleExecutable"], name)
            self.assertEqual(info["CFBundleIdentifier"], "com.colossus.acceptance.helper" + identifier_suffix)
            identifiers.add(info["CFBundleIdentifier"])
            self.assertTrue((helper / "Contents/MacOS" / name).is_file())
        self.assertEqual(len(identifiers), 5)
        inventory = component.verify_installed(self.app, self.app.with_name(self.app.name + ".browser-component.json"))
        self.assertFalse((self.app / component.MANIFEST).exists())
        bundled_dictation = self.app / "Contents/Resources/dictation"
        self.assertEqual(set(path.name for path in bundled_dictation.iterdir()),
                         set(stage_macos.DICTATION_RESOURCES))
        self.assertEqual((bundled_dictation / "ggml-tiny.en.bin").read_text(), "ggml-tiny.en.bin")
        self.assertEqual(inventory["modes"], {"desktop": False, "headless": False})
        signatures = [call.args[0] for call in run.call_args_list if "--sign" in call.args[0]]
        self.assertTrue(all(command[command.index("--sign") + 1] == "-" for command in signatures))
        self.assertTrue(all("--deep" not in command for command in signatures))
        self.assertTrue(signatures[0][-1].endswith(".dylib"))
        self.assertTrue(signatures[3][-1].endswith(stage_macos.FRAMEWORK))
        self.assertTrue(signatures[-1][-1].endswith("Colossus.app"))
        self.assertEqual(sum("--entitlements" in command for command in signatures), 6)
        helper_signatures = signatures[4:-1]
        self.assertTrue(all(command[command.index("--entitlements") + 1] == str(stage_macos.ENTITLEMENTS)
                            for command in helper_signatures))
        self.assertEqual(signatures[-1][signatures[-1].index("--entitlements") + 1],
                         str(stage_macos.MAIN_ENTITLEMENTS))
        with (self.app / "Contents/Info.plist").open("rb") as source:
            main_info = plistlib.load(source)
        self.assertIn("offline dictation", main_info["NSMicrophoneUsageDescription"])
        with stage_macos.ENTITLEMENTS.open("rb") as source:
            helper_entitlements = plistlib.load(source)
        with stage_macos.MAIN_ENTITLEMENTS.open("rb") as source:
            main_entitlements = plistlib.load(source)
        self.assertNotIn("com.apple.security.device.audio-input", helper_entitlements)
        self.assertTrue(main_entitlements["com.apple.security.device.audio-input"])
        # Signing never mutates the pinned source framework or build helpers.
        self.assertFalse((self.source / "Release" / stage_macos.FRAMEWORK / "Versions").exists())

    def test_missing_helper_or_sandbox_resource_prevents_publication(self):
        helper = self.build / "helpers" / (stage_macos.HELPER + " (Renderer).app")
        (helper / "Contents/MacOS" / (stage_macos.HELPER + " (Renderer)")).unlink()
        with patch.object(stage_macos, "run") as run:
            with self.assertRaises((component.ComponentError, OSError)):
                self.stage()
            run.assert_not_called()
        self.assertFalse(self.app.exists())

    def test_signing_failure_cleans_temporary_stage_and_keeps_destination_absent(self):
        with patch.object(stage_macos, "run", side_effect=subprocess.CalledProcessError(1, "codesign")):
            with self.assertRaises(subprocess.CalledProcessError):
                self.stage()
        self.assertFalse(self.app.exists())
        self.assertEqual(list(self.root.glob(".cef-app-stage-*")), [])

    def test_missing_sandbox_library_is_refused_before_signing(self):
        (self.source / "Release" / stage_macos.FRAMEWORK / "Libraries/libcef_sandbox.dylib").unlink()
        with patch.object(stage_macos, "run") as run:
            with self.assertRaises(component.ComponentError):
                self.stage()
            run.assert_not_called()
        self.assertFalse(self.app.exists())

    def test_linked_or_missing_dictation_asset_is_refused_before_staging(self):
        asset = self.dictation / "ggml-tiny.en.bin"
        asset.unlink()
        asset.symlink_to(self.dictation / "models.json")
        with patch.object(stage_macos, "run") as run:
            with self.assertRaises(component.ComponentError):
                self.stage()
            run.assert_not_called()
        self.assertFalse(self.app.exists())

    def test_existing_or_linked_app_is_never_replaced(self):
        self.app.mkdir()
        marker = self.app / "keep"
        marker.write_text("existing app")
        with self.assertRaises(component.ComponentError):
            self.stage()
        self.assertEqual(marker.read_text(), "existing app")
        marker.unlink()
        self.app.rmdir()
        self.app.symlink_to(self.root / "absent.app")
        with self.assertRaises(component.ComponentError):
            self.stage()

    def test_receipt_created_during_signing_is_preserved_and_app_is_rolled_back(self):
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        for kind in ("file", "directory"):
            with self.subTest(kind=kind):
                def introduce_receipt(command):
                    if "--sign" in command and not receipt.exists():
                        if kind == "file":
                            receipt.write_text("unrelated receipt")
                        else:
                            receipt.mkdir()
                            (receipt / "keep").write_text("unrelated directory")

                with patch.object(stage_macos, "run", side_effect=introduce_receipt):
                    with self.assertRaises(FileExistsError):
                        self.stage()
                self.assertFalse(self.app.exists())
                preserved = receipt if kind == "file" else receipt / "keep"
                self.assertEqual(preserved.read_text(), "unrelated receipt" if kind == "file"
                                 else "unrelated directory")
                if kind == "directory":
                    (receipt / "keep").unlink()
                    receipt.rmdir()
                else:
                    receipt.unlink()
                self.assertEqual(list(self.root.glob(".cef-app-stage-*")), [])
                self.assertEqual(list(self.root.glob(".cef-app-rollback-*")), [])

    def test_app_created_at_publication_is_never_replaced(self):
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        rename = stage_macos.rename_exclusive
        for kind in ("directory", "file", "symlink"):
            with self.subTest(kind=kind):
                def introduce_app(source, destination):
                    if destination == self.app:
                        if kind == "directory":
                            self.app.mkdir()
                        elif kind == "file":
                            self.app.write_text("unrelated app file")
                        else:
                            self.app.symlink_to(self.root / "absent.app")
                        identity = stage_macos.object_identity(self.app)
                        with self.assertRaises(FileExistsError):
                            rename(source, destination)
                        self.assertEqual(stage_macos.object_identity(self.app), identity)
                        raise FileExistsError(errno.EEXIST, "raced destination", str(destination))
                    rename(source, destination)

                with patch.object(stage_macos, "run"), \
                        patch.object(stage_macos, "rename_exclusive", side_effect=introduce_app):
                    with self.assertRaises(FileExistsError):
                        self.stage()
                self.assertFalse(receipt.exists())
                if kind == "directory":
                    self.assertEqual(list(self.app.iterdir()), [])
                    self.app.rmdir()
                elif kind == "file":
                    self.assertEqual(self.app.read_text(), "unrelated app file")
                    self.app.unlink()
                else:
                    self.assertTrue(self.app.is_symlink())
                    self.assertEqual(os.readlink(self.app), str(self.root / "absent.app"))
                    self.app.unlink()

    def test_receipt_publication_io_failure_removes_only_the_owned_app(self):
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        rename = stage_macos.rename_exclusive

        def fail_receipt(source, destination):
            if destination == receipt:
                self.assertTrue(self.app.is_dir())
                raise OSError(errno.EIO, "fixture publication failure")
            rename(source, destination)

        with patch.object(stage_macos, "run"), \
                patch.object(stage_macos, "rename_exclusive", side_effect=fail_receipt):
            with self.assertRaises(OSError) as failure:
                self.stage()
        self.assertEqual(failure.exception.errno, errno.EIO)
        self.assertFalse(self.app.exists())
        self.assertFalse(receipt.exists())
        self.assertEqual(list(self.root.glob(".cef-app-stage-*")), [])
        self.assertEqual(list(self.root.glob(".cef-app-rollback-*")), [])

    def test_rollback_preserves_an_app_replaced_before_cleanup(self):
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        moved_app = self.root / "moved-owned.app"
        rename = stage_macos.rename_exclusive

        def replace_app(source, destination):
            if destination == receipt:
                self.app.rename(moved_app)
                self.app.mkdir()
                (self.app / "keep").write_text("unrelated replacement")
                raise OSError(errno.EIO, "fixture publication failure")
            rename(source, destination)

        with patch.object(stage_macos, "run"), \
                patch.object(stage_macos, "rename_exclusive", side_effect=replace_app):
            with self.assertRaises(OSError):
                self.stage()
        self.assertEqual((self.app / "keep").read_text(), "unrelated replacement")
        self.assertTrue((moved_app / "Contents/MacOS/browser-acceptance").is_file())
        self.assertFalse(receipt.exists())

    def test_rollback_rechecks_a_destination_swapped_during_reclaim(self):
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        moved_app = self.root / "moved-owned.app"
        rename = stage_macos.rename_exclusive

        def swap_on_reclaim(source, destination):
            if destination == receipt:
                raise OSError(errno.EIO, "fixture publication failure")
            if source == self.app:
                self.app.rename(moved_app)
                self.app.mkdir()
                (self.app / "keep").write_text("unrelated replacement")
            rename(source, destination)

        with patch.object(stage_macos, "run"), \
                patch.object(stage_macos, "rename_exclusive", side_effect=swap_on_reclaim):
            with self.assertRaises(OSError):
                self.stage()
        self.assertEqual((self.app / "keep").read_text(), "unrelated replacement")
        self.assertEqual(list(self.root.glob(".cef-app-rollback-*")), [])

    def test_rollback_preserves_unrelated_reclaimed_object_if_restore_collides(self):
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        moved_app = self.root / "moved-owned.app"
        rename = stage_macos.rename_exclusive

        def collide_on_restore(source, destination):
            if destination == receipt:
                raise OSError(errno.EIO, "fixture publication failure")
            if source == self.app:
                self.app.rename(moved_app)
                self.app.mkdir()
                (self.app / "keep").write_text("reclaimed unrelated app")
            elif destination == self.app and source.parent.name.startswith(".cef-app-rollback-"):
                self.app.mkdir()
                (self.app / "keep").write_text("latest unrelated app")
            rename(source, destination)

        with patch.object(stage_macos, "run"), \
                patch.object(stage_macos, "rename_exclusive", side_effect=collide_on_restore):
            with self.assertRaises(component.ComponentError) as failure:
                self.stage()
        self.assertEqual((self.app / "keep").read_text(), "latest unrelated app")
        quarantine, = self.root.glob(".cef-app-rollback-*")
        recovered = quarantine / self.app.name
        self.assertEqual((recovered / "keep").read_text(), "reclaimed unrelated app")
        self.assertIn(str(recovered), str(failure.exception))
        self.assertEqual(list(self.root.glob(".cef-app-stage-*")), [])

    def test_failed_later_publication_rolls_back_an_owned_receipt(self):
        source_receipt = self.root / "temporary-receipt.json"
        source_receipt.write_text("owned receipt")
        receipt = self.app.with_name(self.app.name + ".browser-component.json")
        self.app.mkdir()
        identity = stage_macos.object_identity(self.app)
        with self.assertRaises(FileExistsError):
            stage_macos.publish_artifacts([(source_receipt, receipt), (self.source, self.app)])
        self.assertEqual(stage_macos.object_identity(self.app), identity)
        self.assertFalse(receipt.exists())
        self.assertTrue(self.source.is_dir())
        self.assertEqual(list(self.root.glob(".cef-app-rollback-*")), [])

    def test_modified_source_is_rejected_before_copying_or_signing(self):
        self.verifier.stop()
        with patch.object(stage_macos, "run") as run:
            with self.assertRaises(component.ComponentError):
                self.stage()
            run.assert_not_called()
        self.assertFalse(self.app.exists())


if __name__ == "__main__":
    unittest.main()
