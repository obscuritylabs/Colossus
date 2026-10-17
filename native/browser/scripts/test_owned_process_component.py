"""Patched component gates; no CEF load, signing or native process launch."""
import subprocess
import os
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

import component
import owned_process_component as owned


class ExportTests(unittest.TestCase):
    def test_only_actual_defined_exact_private_symbols_satisfy_the_gate(self):
        for output in ("_cef_initialize\n", "_colossus_owned_process_install_extra\n_colossus_owned_helper_install\n"):
            with patch.object(owned.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)), self.assertRaises(component.ComponentError):
                owned.symbols(Path("/unused"), owned.PRIVATE_EXPORTS)
        result = subprocess.CompletedProcess([], 0, "_colossus_owned_process_install\n_colossus_owned_helper_install\n")
        with patch.object(owned.subprocess, "run", return_value=result) as run:
            owned.symbols(Path("/unused"), owned.PRIVATE_EXPORTS)
        self.assertEqual(run.call_args.args[0][1], "-gjU")

    def test_executable_cannot_substitute_for_a_body_dylib(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "Body.dylib"
            image.write_bytes(b"fixture")
            with patch.object(owned.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "MH_MAGIC_64 ARM64 EXECUTE\n")), self.assertRaises(component.ComponentError):
                owned.dylib(image)

    def test_all_architecture_slices_must_have_the_required_image_type(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "Body.dylib"
            image.write_bytes(b"fixture")
            output = "MH_MAGIC_64 ARM64 ALL 0x00 DYLIB\nMH_MAGIC_64 X86_64 ALL 0x00 EXECUTE\n"
            with patch.object(owned.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)), self.assertRaises(component.ComponentError):
                owned.dylib(image)

    def test_parenthesized_helper_is_inspected_through_its_actual_open_descriptor(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "Colossus Browser Helper (Alerts)"
            image.write_bytes(b"actual opened image")

            def inspect(command, **options):
                descriptor, = options["pass_fds"]
                self.assertEqual(command[-1], f"/dev/fd/{descriptor}")
                self.assertEqual(os.fstat(descriptor).st_ino, image.stat().st_ino)
                self.assertEqual(os.read(descriptor, 32), b"actual opened image")
                return subprocess.CompletedProcess(command, 0, "MH_MAGIC_64 ARM64 ALL 0x00 EXECUTE\n")

            with patch.object(owned.subprocess, "run", side_effect=inspect):
                owned.macho_type(image, "EXECUTE")

    def test_owned_export_failures_are_not_stock_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "include").mkdir()
            (root / "include/cef_version.h").write_text('#define CEF_VERSION "' + component.CEF_VERSION + '"\n')
            with patch.object(owned, "symbols", side_effect=component.ComponentError("private ABI absent")), self.assertRaises(component.ComponentError):
                owned.metadata(root, "macosarm64")


class NativeBuildReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        temporary = Path(self.temporary.name).resolve()
        self.root = temporary / "cef"
        self.root.mkdir()
        (self.root / owned.SOURCE_RECEIPT).write_text("{}\n")
        self.native = temporary / "native"
        self.native.mkdir()
        metadata = {"schema_version": 1, "development_only": True,
                    "production_accepted": False, "native_acceptance": False,
                    "platform": "macosarm64", "framework_sha256": "a" * 64}
        for name, kind in owned.NATIVE_ARTIFACTS:
            image = self.native / name
            image.parent.mkdir(parents=True, exist_ok=True)
            image.write_bytes(("fixture " + kind + " " + name).encode())
            image.chmod(0o755 if kind != "archive" else 0o644)
        self.image_types = {name: "EXECUTE" if kind == "front" else "DYLIB"
                            for name, kind in owned.NATIVE_ARTIFACTS if kind != "archive"}
        self.missing_symbols = set()
        for patcher in (patch.object(owned, "verify_source", return_value=self.root),
                        patch.object(owned, "metadata", return_value=metadata),
                        patch.object(owned.subprocess, "run", side_effect=self.native_tool_output)):
            patcher.start()
            self.addCleanup(patcher.stop)
        owned.write_build_configuration(self.root, self.native, "macosarm64")

    def native_tool_output(self, command, **_options):
        image = Path(command[-1])
        if command[0] == "/usr/bin/otool":
            descriptor, = _options["pass_fds"]
            info = os.fstat(descriptor)
            image = next(self.native / name for name, _kind in owned.NATIVE_ARTIFACTS
                         if (self.native / name).stat().st_ino == info.st_ino)
        relative = image.relative_to(self.native).as_posix()
        if command[0] == "/usr/bin/otool":
            output = "MH_MAGIC_64 ARM64 ALL 0x00 " + self.image_types[relative] + "\n"
        else:
            required = (owned.MAIN_INSTALL_EXPORT, owned.BODY_EXPORT, *owned.FRONT_EXPORTS)
            output = "".join("_" + name + "\n" for name in required
                             if (relative, name) not in self.missing_symbols)
        return subprocess.CompletedProcess(command, 0, output)

    def test_configure_only_cannot_satisfy_completed_build_gate(self):
        with self.assertRaises(component.ComponentError), patch.object(owned.subprocess, "run") as run:
            owned.verify_build(self.root, self.native, "macosarm64")
        run.assert_not_called()

    @unittest.skipUnless(os.name == "posix", "POSIX executable modes; Linux/macOS CI contracts")
    def test_completed_receipt_binds_twelve_outputs_without_the_external_rust_main(self):
        receipt = owned.write_native_build(self.root, self.native, "macosarm64")
        metadata = owned.verify_build(self.root, self.native, "macosarm64")
        self.assertEqual(metadata["native_artifact_receipt_sha256"], component.digest_file(receipt))
        completed = component.load_json(receipt)
        self.assertEqual(len(completed["files"]), 12)
        self.assertEqual(sum(record["kind"] == "front" for record in completed["files"]), 6)
        self.assertEqual(sum(record["kind"] == "body" for record in completed["files"]), 5)
        self.assertFalse((self.native / "libcolossus_native_browser_host.dylib").exists())
        self.assertFalse(completed["production_accepted"])
        self.assertFalse(completed["native_acceptance"])

    @unittest.skipUnless(os.name == "posix", "POSIX executable modes; Linux/macOS CI contracts")
    def test_modified_archive_or_any_front_or_helper_body_is_rejected(self):
        owned.write_native_build(self.root, self.native, "macosarm64")
        for relative, _kind in owned.NATIVE_ARTIFACTS:
            with self.subTest(relative=relative):
                image = self.native / relative
                original = image.read_bytes()
                image.write_bytes(original + b" changed")
                with self.assertRaises(component.ComponentError):
                    owned.verify_build(self.root, self.native, "macosarm64")
                image.write_bytes(original)

    @unittest.skipUnless(os.name == "posix", "POSIX executable modes; Linux/macOS CI contracts")
    def test_wrong_front_or_body_type_cannot_receive_a_completed_receipt(self):
        for relative, kind in owned.NATIVE_ARTIFACTS:
            if kind == "archive":
                continue
            with self.subTest(relative=relative):
                original = self.image_types[relative]
                self.image_types[relative] = "DYLIB" if kind == "front" else "EXECUTE"
                with self.assertRaises(component.ComponentError):
                    owned.write_native_build(self.root, self.native, "macosarm64")
                self.image_types[relative] = original
                self.assertFalse((self.native / owned.NATIVE_BUILD_RECEIPT).exists())

    def test_archive_requires_the_actual_private_main_installer(self):
        self.missing_symbols.add(("libcolossus_cef.a", owned.MAIN_INSTALL_EXPORT))
        with self.assertRaises(component.ComponentError):
            owned.write_native_build(self.root, self.native, "macosarm64")
        self.assertFalse((self.native / owned.NATIVE_BUILD_RECEIPT).exists())

    @unittest.skipUnless(os.name == "posix", "POSIX executable modes; Linux/macOS CI contracts")
    def test_stock_front_cannot_substitute_for_the_owned_loader(self):
        self.missing_symbols.add(("envelope/colossus-native-browser-host", owned.FRONT_EXPORTS[0]))
        with self.assertRaises(component.ComponentError):
            owned.write_native_build(self.root, self.native, "macosarm64")
        self.assertFalse((self.native / owned.NATIVE_BUILD_RECEIPT).exists())

    @unittest.skipUnless(os.name == "posix", "POSIX executable modes; Linux/macOS CI contracts")
    def test_native_source_change_invalidates_the_configuration(self):
        owned.write_native_build(self.root, self.native, "macosarm64")
        with patch.object(owned, "native_source_hashes", return_value={"changed": "b" * 64}), self.assertRaises(component.ComponentError):
            owned.verify_build(self.root, self.native, "macosarm64")

    @unittest.skipUnless(os.name == "posix", "POSIX executable modes; Linux/macOS CI contracts")
    def test_rebuilt_outputs_need_a_new_complete_receipt(self):
        receipt = owned.write_native_build(self.root, self.native, "macosarm64")
        before = receipt.read_bytes()
        image = self.native / "libcolossus_cef.a"
        image.write_bytes(image.read_bytes() + b" rebuilt")
        with self.assertRaises(component.ComponentError):
            owned.verify_build(self.root, self.native, "macosarm64")
        owned.write_native_build(self.root, self.native, "macosarm64")
        self.assertNotEqual(receipt.read_bytes(), before)
        owned.verify_build(self.root, self.native, "macosarm64")


if __name__ == "__main__":
    unittest.main()
