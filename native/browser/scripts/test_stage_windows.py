"""Windows bootstrap/client packaging boundaries; no Windows execution is mocked."""

from pathlib import Path
import base64
import hashlib
import json
import os
import subprocess
import struct
import tempfile
import unittest
from unittest.mock import patch

import component
import pki_fixture_private
import stage_windows
import windows_preview


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
    # IMAGE_EXPORT_DIRECTORY has distinct function and name RVA tables at
    # offsets 28 and 32. Use a real function RVA, not an export-name string, in
    # the function table so the fixture cannot mask reading the wrong field.
    struct.pack_into("<IIIII", image, 0x200 + 20, 1, 1, 0x1080, 0x1040, 0x1060)
    struct.pack_into("<I", image, 0x280, 0x1300)
    image[0x500] = 0xc3
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

    @unittest.skipUnless(os.name == "nt", "real Windows component ACL contract")
    def test_component_preserves_explicit_user_access_and_scopes_package_reads(self):
        # Use the real stager, including its nested TemporaryDirectory and rename.
        # All fixture PE bytes are parsed/copied only, never executed.
        self.acl.stop()
        sibling = self.root / "private-profile"
        pki_fixture_private.private_directory(sibling)
        private_file = sibling / "private.txt"
        private_file.write_text("fixture only")
        untouched = [self.root, self.source, sibling, private_file]
        before = windows_acls(untouched)
        stage_windows.stage(self.source, self.native_build, self.client, self.destination)
        self.assertEqual(before, windows_acls(untouched))
        staged = windows_acls([self.destination, self.destination / "chrome_elf.dll",
                              self.destination / "locales/en-US.pak"])
        self.assertTrue(staged[0]["protected"])
        for index, item in enumerate(staged):
            self.assertEqual(len(item["user"]), 1)
            self.assertEqual(item["user"][0]["mask"], 0x1f01ff)
            self.assertEqual(item["user"][0]["type"], "Allow")
            self.assertEqual(item["user"][0]["inherited"], index != 0)
            self.assertEqual({rule["sid"] for rule in item["packages"]},
                             {"S-1-15-2-1", "S-1-15-2-2"})
            self.assertEqual(len(item["packages"]), 2)
            for rule in item["packages"]:
                self.assertEqual(rule["mask"], 0x1200a9)
                self.assertEqual(rule["type"], "Allow")
                self.assertEqual(rule["inherited"], index != 0)

    def test_rejects_wrong_architecture_export_eager_or_missing_delayed_cef(self):
        for options in ({"machine": 0xaa64}, {"export": b"WrongEntry"}, {"eager": True}, {"delayed": False}):
            with self.subTest(options=options):
                self.client.write_bytes(client_image(**options))
                with self.assertRaises(component.ComponentError):
                    stage_windows.stage(self.source, self.native_build, self.client, self.destination)
                self.assertFalse(self.destination.exists())

    def test_function_table_bytes_cannot_substitute_for_a_named_bootstrap_export(self):
        image = client_image(export=b"WrongEntry")
        # An incorrect parser reading AddressOfFunctions as AddressOfNames would
        # accept this string even though no export has the required name.
        struct.pack_into("<I", image, 0x280, 0x1090)
        image[0x290:0x29b] = b"RunWinMain\0"
        self.client.write_bytes(image)
        with self.assertRaises(component.ComponentError):
            stage_windows.inspect_client(self.client)

    def test_private_host_has_fixed_bootstrap_name_without_desktop_resources(self):
        program = stage_windows.stage(self.source, self.native_build, self.client, self.destination, host=True)
        self.assertEqual(program.name, "colossus-native-browser-host.exe")
        self.assertEqual(program.with_suffix(".dll").read_bytes(), self.client.read_bytes())
        self.assertFalse((self.destination / "colossus-chromium-preview.exe").exists())
        self.assertFalse((self.destination / "dictation").exists())
        self.assertEqual(component.verify_installed(self.destination)["modes"], {"desktop": False, "headless": False})

    def test_publisher_host_can_only_be_sealed_after_its_final_binary_transformation(self):
        stage_windows.stage(self.source, self.native_build, self.client, self.destination,
                            host=True, defer_inventory=True)
        self.assertFalse((self.destination / "browser-component.json").exists())
        output = self.root / "preview-seal.json"
        with self.assertRaises((OSError, component.ComponentError)):
            windows_preview.seal(self.destination, output)
        # A signing transform changes file bytes. The fresh inventory must bind
        # those final bytes; this synthetic fixture does not assert a signature.
        host = self.destination / "colossus-native-browser-host.dll"
        host.write_bytes(host.read_bytes() + b"publisher-transform-fixture")
        component.installed_inventory(self.destination, "windows64", "colossus-native-browser-host.exe")
        windows_preview.seal(self.destination, output)
        seal = json.loads(output.read_text())
        self.assertEqual(seal["component_path"], "browser/component")
        self.assertNotIn(str(self.root), output.read_text())
        manifest = self.destination / "browser-component.json"
        self.assertEqual(seal["component_manifest_sha256"], hashlib.sha256(manifest.read_bytes()).hexdigest())
        self.assertFalse(seal["production_acceptance"])
        self.assertEqual(seal["capabilities"], {
            "embedded": True, "headless": False, "restrictive_egress": False,
            "private_ca_trust": False, "client_identities": False})
        with self.assertRaises(FileExistsError):
            windows_preview.seal(self.destination, output)
        host.write_bytes(host.read_bytes() + b"late-change")
        with self.assertRaises(component.ComponentError):
            windows_preview.seal(self.destination, self.root / "changed-seal.json")

    def test_deferred_inventory_never_applies_to_an_in_process_preview(self):
        with self.assertRaises(component.ComponentError):
            stage_windows.stage(self.source, self.native_build, self.client, self.destination,
                                defer_inventory=True)
        self.assertFalse(self.destination.exists())

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
                    # macOS resolves /var through /private/var while the test
                    # fixture records the original spelling of the same file.
                    if Path(source).samefile(changed):
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



def windows_acls(paths):
    """Read actual kernel DACLs; return hashes and closed user/package categories."""
    script = r'''
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$paths = ConvertFrom-Json $env:COLOSSUS_ACL_TEST_PATHS
$userSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$sha = [Security.Cryptography.SHA256]::Create()
try {
  $results = @(foreach ($path in $paths) {
    $acl = [IO.File]::GetAccessControl($path, [Security.AccessControl.AccessControlSections]::Access)
    $dacl = $acl.GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]::Access)
    $digest = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($dacl))).Replace('-', '')
    $rules = @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
    $user = @($rules | Where-Object { $_.IdentityReference.Value -eq $userSid } | ForEach-Object {
      [ordered]@{mask=[long]$_.FileSystemRights; type=[string]$_.AccessControlType; inherited=$_.IsInherited}
    })
    $packages = @($rules | ForEach-Object {
      $sid = $_.IdentityReference.Value
      if ($sid -eq 'S-1-15-2-1' -or $sid -eq 'S-1-15-2-2') {
        [ordered]@{sid=$sid; mask=[long]$_.FileSystemRights; type=[string]$_.AccessControlType; inherited=$_.IsInherited}
      }
    })
    [ordered]@{digest=$digest; protected=$acl.AreAccessRulesProtected; user=$user; packages=$packages}
  })
  ConvertTo-Json -InputObject $results -Depth 5 -Compress
} finally { $sha.Dispose() }
'''
    powershell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    environment = dict(os.environ, COLOSSUS_ACL_TEST_PATHS=json.dumps([str(p) for p in paths]))
    environment.pop("PSModulePath", None)
    result = subprocess.run(
        [str(powershell), "-NoProfile", "-NonInteractive", "-EncodedCommand",
         base64.b64encode(script.encode("utf-16-le")).decode("ascii")],
        env=environment, stdin=subprocess.DEVNULL, capture_output=True, timeout=30,
        check=False, creationflags=subprocess.CREATE_NO_WINDOW,
    )
    if result.returncode or result.stderr or len(result.stdout) > 16384:
        raise AssertionError("bounded native ACL readback failed")
    values = json.loads(result.stdout)
    if len(values) != len(paths):
        raise AssertionError("native ACL readback returned an unexpected object count")
    return values

if __name__ == "__main__":
    unittest.main()
