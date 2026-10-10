"""Actual ELF structure fixtures for Chromium's RTLD_NEXT libc requirement."""
import os
import json
from pathlib import Path
import struct
import tempfile
import unittest

from elf_order import ElfOrderError, verify_chromium_link_order
import component


def executable(libraries: list[str]) -> bytearray:
    """Minimal ELF64 with program headers and an independently mapped dynstr."""
    strings, offsets = bytearray(b"\0"), []
    for library in libraries:
        offsets.append(len(strings))
        strings.extend(library.encode() + b"\0")
    image = bytearray(2048)
    header = (b"\x7fELF\x02\x01\x01" + b"\0" * 9, 3, 62, 1, 0, 64, 0, 0, 64, 56, 2, 0, 0, 0)
    struct.pack_into("<16sHHIQQQIHHHHHH", image, 0, *header)
    struct.pack_into("<IIQQQQQQ", image, 64, 1, 4, 0, 0x400000, 0, len(image), len(image), 4096)
    entries = [(1, offset) for offset in offsets] + [(5, 0x400400), (10, len(strings)), (0, 0)]
    struct.pack_into("<IIQQQQQQ", image, 120, 2, 4, 512, 0x400200, 0, len(entries) * 16, len(entries) * 16, 8)
    for index, entry in enumerate(entries):
        struct.pack_into("<QQ", image, 512 + index * 16, *entry)
    image[1024:1024 + len(strings)] = strings
    return image


class ElfDependencyOrder(unittest.TestCase):
    def inspect(self, image):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "native-host"
            path.write_bytes(image)
            verify_chromium_link_order(path)

    def test_normal_chromium_link_order_and_unrelated_dependencies(self):
        self.inspect(executable(["libcef.so", "libstdc++.so.6", "libm.so.6", "libc.so.6"]))

    def test_rust_late_native_flags_reproduce_actual_close_symbol_failure(self):
        for libraries in (["libgcc_s.so.1", "libm.so.6", "libc.so.6", "libcef.so"],
                          ["libc.so.6"], ["libcef.so"],
                          ["libcef.so", "libc.so.6", "libcef.so"]):
            with self.subTest(libraries=libraries), self.assertRaises(ElfOrderError):
                self.inspect(executable(libraries))

    def test_truncated_or_unmapped_structures_never_establish_order(self):
        original = executable(["libcef.so", "libc.so.6"])
        for length in (0, 63, 119, 511, 550, 1024):
            with self.subTest(length=length), self.assertRaises(ElfOrderError):
                self.inspect(original[:length])
        mutations = [(5, "<Q", 2**64 - 8), (10, "<H", 257), (9, "<H", 48)]
        for field, encoding, value in mutations:
            image = original.copy()
            header_offsets = {5: 32, 9: 54, 10: 56}
            struct.pack_into(encoding, image, header_offsets[field], value)
            with self.subTest(field=field), self.assertRaises(ElfOrderError):
                self.inspect(image)
        image = original.copy()
        struct.pack_into("<Q", image, 512 + 2 * 16 + 8, 0x900000)
        with self.assertRaises(ElfOrderError):
            self.inspect(image)

    def test_missing_null_termination_and_oversized_strings_fail(self):
        original = executable(["libcef.so", "libc.so.6"])
        for offset, tag, value in [(512 + 4 * 16, 1, 1), (512 + 3 * 16, 10, 2**20 + 1),
                                   (512, 1, 2048), (512 + 2 * 16, 5, 0x4007F0)]:
            image = original.copy()
            struct.pack_into("<QQ", image, offset, tag, value)
            with self.subTest(offset=offset), self.assertRaises(ElfOrderError):
                self.inspect(image)
        image = original.copy()
        image[1024:1060] = b"x" * 36
        with self.assertRaises(ElfOrderError):
            self.inspect(image)

    def test_bad_host_cannot_get_a_component_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            host = root / "colossus-native-browser-host"
            host.write_bytes(executable(["libc.so.6", "libcef.so"]))
            host.chmod(0o755)
            with self.assertRaisesRegex(component.ComponentError, "libcef before libc"):
                component.installed_inventory(root, "linux64", host.name)
            self.assertFalse((root / component.MANIFEST).exists())

    def test_consistent_hashes_alone_cannot_verify_unusable_host_order(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            host = root / "colossus-native-browser-host"
            host.write_bytes(executable(["libcef.so", "libc.so.6"]))
            host.chmod(0o755)
            manifest = component.installed_inventory(root, "linux64", host.name)
            component.verify_installed(root)
            host.write_bytes(executable(["libc.so.6", "libcef.so"]))
            document = json.loads(manifest.read_text())
            document["files"] = component.inventory_files(root, component.MANIFEST)
            manifest.write_text(json.dumps(document))
            with self.assertRaisesRegex(component.ComponentError, "libcef before libc"):
                component.verify_installed(root)

    @unittest.skipIf(os.name == "nt", "POSIX no-follow source custody")
    def test_symlinked_executable_is_not_inspected(self):
        with tempfile.TemporaryDirectory() as directory:
            actual, link = Path(directory) / "actual", Path(directory) / "link"
            actual.write_bytes(executable(["libcef.so", "libc.so.6"]))
            link.symlink_to(actual)
            with self.assertRaises(OSError):
                verify_chromium_link_order(link)


if __name__ == "__main__":
    unittest.main()
