"""Windows bootstrap/client packaging boundaries; no Windows execution is mocked."""

from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import component
import stage_windows


def client_image(*, export=b"RunWinMain", eager=False, machine=0x8664, delayed=True):
    image = bytearray(4096)
    image[:2] = b"MZ"
    struct.pack_into("<I", image, 60, 0x80)
    image[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HH", image, 0x84, machine, 1)
    struct.pack_into("<HH", image, 0x94, 240, 0x2000)
    struct.pack_into("<H", image, 0x98, 0x20b)
    struct.pack_into("<I", image, 0x98 + 108, 16)
    section = 0x98 + 240
    struct.pack_into("<IIII", image, section + 8, 2048, 0x1000, 2048, 0x200)
    struct.pack_into("<II", image, 0x98 + 112, 0x1000, 64)
    struct.pack_into("<II", image, 0x200 + 24, 1, 0x1040)
    struct.pack_into("<I", image, 0x240, 0x1050)
    image[0x250:0x250 + len(export) + 1] = export + b"\0"
    if delayed:
        struct.pack_into("<II", image, 0x98 + 112 + 13 * 8, 0x1100, 64)
        struct.pack_into("<II", image, 0x300, 1, 0x1150)
        image[0x350:0x35b] = b"libcef.dll\0"
    if eager:
        struct.pack_into("<II", image, 0x98 + 112 + 8, 0x1200, 40)
        struct.pack_into("<I", image, 0x400 + 12, 0x1250)
        image[0x450:0x45b] = b"libcef.dll\0"
    return image


class WindowsStageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        for name in ("bootstrap.exe", "chrome_elf.dll", "libcef.dll", "v8_context_snapshot.bin", "libcef.lib"):
            path = self.source / "Release" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        for name in (*stage_windows.RESOURCES, "locales/en-US.pak"):
            path = self.source / "Resources" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        for name in ("LICENSE.txt", "CREDITS.html"):
            (self.source / name).write_text(name)
        self.client = self.root / "client.dll"
        self.client.write_bytes(client_image())
        self.native_build = self.root / "native"
        self.native_build.mkdir()
        (self.native_build / "colossus-browser-helper.dll").write_bytes(client_image())
        self.destination = self.root / "preview"
        self.verify = patch.object(stage_windows.stage_macos, "verify_source", return_value=self.source)
        self.verify.start()
        self.addCleanup(self.verify.stop)
        self.acl = patch.object(stage_windows, "grant_lpac_read")
        self.grant = self.acl.start()
        self.addCleanup(self.acl.stop)

    def test_complete_bootstrap_client_resources_inventory_without_promoting_modes(self):
        source_before = component.inventory_files(self.source, None)
        program = stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertEqual(program.name, "colossus-chromium-preview.exe")
        self.assertEqual(program.read_bytes(), b"bootstrap.exe")
        self.assertEqual(program.with_suffix(".dll").read_bytes(), self.client.read_bytes())
        self.assertEqual((self.destination / "colossus-browser-helper.exe").read_bytes(), b"bootstrap.exe")
        self.assertEqual((self.destination / "colossus-browser-helper.dll").read_bytes(), client_image())
        self.assertFalse((self.destination / "libcef.lib").exists())
        self.assertTrue((self.destination / "locales/en-US.pak").is_file())
        self.assertEqual(component.verify_installed(self.destination)["modes"], {"desktop": False, "headless": False})
        self.assertEqual(component.inventory_files(self.source, None), source_before)
        self.grant.assert_called_once()

    def test_rejects_wrong_architecture_export_eager_or_missing_delayed_cef(self):
        for options in ({"machine": 0xaa64}, {"export": b"WrongEntry"}, {"eager": True}, {"delayed": False}):
            with self.subTest(options=options):
                self.client.write_bytes(client_image(**options))
                with self.assertRaises(component.ComponentError):
                    stage_windows.stage(self.source, self.native_build, self.client, self.destination)
                self.assertFalse(self.destination.exists())

    def test_private_host_has_fixed_bootstrap_name_without_desktop_resources(self):
        program = stage_windows.stage(self.source, self.native_build, self.client, self.destination, host=True)
        self.assertEqual(program.name, "colossus-native-browser-host.exe")
        self.assertEqual(program.with_suffix(".dll").read_bytes(), self.client.read_bytes())
        self.assertFalse((self.destination / "colossus-chromium-preview.exe").exists())
        self.assertFalse((self.destination / "dictation").exists())
        self.assertEqual(component.verify_installed(self.destination)["modes"], {"desktop": False, "headless": False})

    def test_private_host_rejects_desktop_resources(self):
        with self.assertRaises(component.ComponentError):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination,
                                self.root, host=True)
        self.assertFalse(self.destination.exists())
        self.grant.assert_not_called()

    def test_rejects_outside_or_unbounded_pe_addresses(self):
        image = client_image()
        struct.pack_into("<I", image, 0x240, 0xffffffff)
        self.client.write_bytes(image)
        with self.assertRaises(component.ComponentError):
            stage_windows.inspect_client(self.client)

    def test_rejects_undeclared_or_truncated_import_directories(self):
        for directory_count, eager_size, delay_size in (
                (0, 0, 64), (13, 0, 64), (17, 0, 64),
                (16, 1, 64), (16, 19, 64), (16, 20, 64),
                (16, 0, 1), (16, 0, 31), (16, 0, 32)):
            with self.subTest(directory_count=directory_count, eager_size=eager_size,
                              delay_size=delay_size):
                image = client_image()
                struct.pack_into("<I", image, 0x98 + 108, directory_count)
                struct.pack_into("<II", image, 0x98 + 112 + 13 * 8, 0x1100, delay_size)
                if eager_size:
                    struct.pack_into("<II", image, 0x98 + 112 + 8, 0x1200, eager_size)
                    struct.pack_into("<I", image, 0x400 + 12, 0x1250)
                    image[0x450:0x45d] = b"kernel32.dll\0"
                self.client.write_bytes(image)
                with self.assertRaises(component.ComponentError):
                    stage_windows.inspect_client(self.client)

    def test_rechecks_copied_clients_before_acl_or_publication(self):
        original_copy = stage_windows.shutil.copy2
        for changed in (self.client, self.native_build / "colossus-browser-helper.dll"):
            with self.subTest(changed=changed):
                def replace_during_copy(source, target, **options):
                    copied = original_copy(source, target, **options)
                    if Path(source) == changed:
                        Path(target).write_bytes(client_image(eager=True))
                    return copied
                with patch.object(stage_windows.shutil, "copy2", side_effect=replace_during_copy):
                    with self.assertRaises(component.ComponentError):
                        stage_windows.stage(self.source, self.native_build, self.client, self.destination)
                self.assertFalse(self.destination.exists())
                self.grant.assert_not_called()

    def test_missing_resource_prevents_publication(self):
        (self.source / "Resources/icudtl.dat").unlink()
        with self.assertRaises((component.ComponentError, OSError)):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertFalse(self.destination.exists())
        self.grant.assert_not_called()

    def test_invalid_native_helper_prevents_publication(self):
        (self.native_build / "colossus-browser-helper.dll").write_bytes(client_image(export=b"WrongEntry"))
        with self.assertRaises(component.ComponentError):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertFalse(self.destination.exists())
        self.grant.assert_not_called()

    def test_existing_destination_is_preserved(self):
        self.destination.mkdir()
        sentinel = self.destination / "keep"
        sentinel.write_text("existing")
        with self.assertRaises(component.ComponentError):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertEqual(sentinel.read_text(), "existing")

    def test_acl_failure_removes_private_stage_without_publishing(self):
        self.grant.side_effect = OSError("ACL refused")
        with self.assertRaises(OSError):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertFalse(self.destination.exists())
        self.assertFalse(list(self.root.glob(".cef-windows-stage-*")))

    def test_concurrent_destination_creation_is_preserved(self):
        def collision(_):
            self.destination.mkdir()
            (self.destination / "keep").write_text("raced")
        self.grant.side_effect = collision
        with self.assertRaises(OSError):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertEqual((self.destination / "keep").read_text(), "raced")

    def test_symlink_client_is_refused(self):
        original = self.client
        link = self.root / "link.dll"
        try:
            link.symlink_to(original)
        except OSError:
            self.skipTest("symlink creation unavailable on this Windows checkout")
        with self.assertRaises(component.ComponentError):
            stage_windows.stage(self.source, self.native_build, link, self.destination)


if __name__ == "__main__":
    unittest.main()
