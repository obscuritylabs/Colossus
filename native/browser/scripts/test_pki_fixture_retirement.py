"""Meaningful retained-inode and ordered pre-CEF acknowledgement checks."""

import os
from pathlib import Path
import socket
import tempfile
import threading
import unittest

from pki_fixture_private import private_write
from pki_fixture_retirement import FixtureInputs, startup_receipt
from run_probe import ProbeError


class RetirementTests(unittest.TestCase):
    def test_retirement_removes_exact_created_sources_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as allocation:
            root = Path(allocation)
            source = root / "fixture-source"
            private_write(source, b"generated-synthetic-identity")
            with FixtureInputs(root / "inputs") as inputs:
                copied = Path(inputs.copy(source, "identity.pfx", 128))
                self.assertNotEqual(source.stat().st_ino, copied.stat().st_ino)
                self.assertEqual(copied.read_bytes(), source.read_bytes())
                inputs.retire()
                inputs.retire()
                self.assertFalse(copied.exists())
                self.assertTrue(source.exists())
                self.assertEqual(list(inputs.directory.iterdir()), [])

    def test_unknown_replacement_is_preserved_and_cannot_acknowledge(self):
        with tempfile.TemporaryDirectory() as allocation:
            root = Path(allocation)
            source = root / "source"
            private_write(source, b"synthetic-identity")
            with FixtureInputs(root / "inputs") as inputs:
                original = Path(inputs.copy(source, "identity.pfx", 128))
                retained = inputs.directory / "original"
                original.rename(retained)
                private_write(original, b"unknown-replacement")
                with self.assertRaises(ProbeError):
                    inputs.retire()
                self.assertTrue(inputs.quarantined)
                self.assertEqual(retained.read_bytes(), b"synthetic-identity")
                quarantines = list(inputs.directory.glob(".retired-input-*"))
                self.assertEqual(len(quarantines), 1)
                self.assertEqual(quarantines[0].read_bytes(), b"unknown-replacement")
                with self.assertRaises(ProbeError):
                    inputs.retire()
                self.assertTrue(quarantines[0].exists())

    def test_source_bounds_and_indirection_fail_before_copy(self):
        with tempfile.TemporaryDirectory() as allocation:
            root = Path(allocation)
            source = root / "source"
            private_write(source, b"synthetic-password")
            with FixtureInputs(root / "inputs") as inputs:
                with self.assertRaises(ProbeError):
                    inputs.copy(source, "password", 3)
                hardlink = root / "hardlink"
                os.link(source, hardlink)
                with self.assertRaises(ProbeError):
                    inputs.copy(source, "password", 128)
                hardlink.unlink()
                source.chmod(0o644)
                with self.assertRaises(ProbeError):
                    inputs.copy(source, "password", 128)
                self.assertEqual(list(inputs.directory.iterdir()), [])

    def test_native_phase4_receives_ack_only_after_sources_are_missing(self):
        with tempfile.TemporaryDirectory() as allocation:
            root = Path(allocation)
            source = root / "source"
            private_write(source, b"synthetic-password")
            with FixtureInputs(root / "inputs") as inputs:
                copied = Path(inputs.copy(source, "password", 128))
                parent, child = socket.socketpair()
                errors = []
                def native():
                    try:
                        child.settimeout(2)
                        for phase in range(1, 8):
                            child.sendall(b"CBH\x01" + bytes([phase]))
                            if phase == 4:
                                self.assertEqual(child.recv(5), b"CBH\x01\x84")
                                self.assertFalse(copied.exists())
                    except BaseException as error:
                        errors.append(error)
                    finally:
                        child.close()
                thread = threading.Thread(target=native)
                thread.start()
                try:
                    parent.settimeout(2)
                    startup_receipt(parent, inputs)
                finally:
                    parent.close()
                    thread.join(timeout=3)
                self.assertFalse(thread.is_alive())
                self.assertEqual(errors, [])

    def test_invalid_phase_never_sends_ack(self):
        for rejected in (b"CBH\x01\x04", b"CBH\x01\x81", b"CBX\x01\x01"):
            with tempfile.TemporaryDirectory() as allocation:
                with FixtureInputs(Path(allocation) / "inputs") as inputs:
                    parent, child = socket.socketpair()
                    try:
                        child.sendall(rejected)
                        with self.assertRaises(ProbeError):
                            startup_receipt(parent, inputs)
                        child.settimeout(0.02)
                        with self.assertRaises(TimeoutError):
                            child.recv(5)
                    finally:
                        parent.close()
                        child.close()

    def test_phase4_replacement_withholds_ack_and_preserves_unknown_input(self):
        with tempfile.TemporaryDirectory() as allocation:
            root = Path(allocation)
            source = root / "source"
            private_write(source, b"synthetic-identity")
            with FixtureInputs(root / "inputs") as inputs:
                copied = Path(inputs.copy(source, "identity.pfx", 128))
                copied.rename(inputs.directory / "original")
                private_write(copied, b"unknown-replacement")
                parent, child = socket.socketpair()
                try:
                    child.sendall(b"".join(b"CBH\x01" + bytes([phase]) for phase in range(1, 5)))
                    with self.assertRaises(ProbeError):
                        startup_receipt(parent, inputs)
                    child.settimeout(0.02)
                    with self.assertRaises(TimeoutError):
                        child.recv(5)
                    quarantines = list(inputs.directory.glob(".retired-input-*"))
                    self.assertEqual(len(quarantines), 1)
                    self.assertEqual(quarantines[0].read_bytes(), b"unknown-replacement")
                finally:
                    parent.close()
                    child.close()


if __name__ == "__main__":
    unittest.main()
