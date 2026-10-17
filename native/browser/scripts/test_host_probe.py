"""Portable integrity checks for the native acceptance client's private framing."""
import struct
import os
import signal
import subprocess
import sys
import threading
import unittest
from unittest.mock import patch

from run_host_probe import BoundedCapture, Channel, MAX_FRAME, ProbeError, encoded, MAX_OUTPUT_BYTES, startup_receipt


class Stream:
    def __init__(self, response):
        self.response = bytearray(response)
        self.writes = bytearray()

    def recv(self, size):
        count = min(size, 3, len(self.response))
        result = self.response[:count]
        del self.response[:count]
        return bytes(result)

    def sendall(self, value):
        self.writes.extend(value)


class HostFrameTests(unittest.TestCase):
    def test_fragmented_startup_requires_all_native_initialization_phases(self):
        startup_receipt(Stream(b"".join(b"CBH\x01" + bytes([phase]) for phase in range(1, 8))))

    def test_failed_truncated_or_out_of_order_startup_never_admits_bridge(self):
        valid = b"CBH\x01\x01"
        for sequence in (valid, valid + b"CBH\x01\x81", valid + b"CBH\x01\x03", b"CBH\x02\x01"):
            with self.assertRaises(ProbeError):
                startup_receipt(Stream(sequence))

    def frame(self, *, role=b"data", sequence=1, tamper=False):
        channel = Channel(Stream(b""), b"k" * 32, {"instance_nonce":[1] * 16}, role)
        payload = encoded({"result":"ready"})
        tag = channel.tag(b"response", sequence, payload)
        if tamper:
            tag = bytes([tag[0] ^ 1]) + tag[1:]
        return struct.pack(">IQ", len(payload), sequence) + tag + payload

    def client(self, frame):
        return Channel(Stream(frame), b"k" * 32, {"instance_nonce":[1] * 16}, b"data")

    def test_fragmented_authenticated_response(self):
        client = self.client(self.frame())
        self.assertEqual(client.request({"operation":"ready"}), {"result":"ready"})
        self.assertGreater(len(client.stream.writes), 44)

    def test_tamper_and_channel_role_fail_before_typed_data_release(self):
        for frame in (self.frame(tamper=True), self.frame(role=b"control")):
            with self.assertRaises(ProbeError):
                self.client(frame).request({"operation":"ready"})

    def test_replayed_sequence_and_oversized_frame_fail_before_body_allocation(self):
        for frame in (self.frame(sequence=2), struct.pack(">IQ", MAX_FRAME + 1, 1)):
            with self.assertRaises(ProbeError):
                self.client(frame).request({"operation":"ready"})

    def test_lost_private_channel_does_not_acknowledge_native_readiness(self):
        with self.assertRaises(ProbeError):
            self.client(b"").request({"operation":"ready"})


@unittest.skipUnless(os.name == "posix", "native process group acceptance requires POSIX")
class HostOutputBounds(unittest.TestCase):
    def child(self, source):
        return subprocess.Popen([sys.executable, "-c", source], stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)

    def test_both_streams_are_drained_before_acknowledged_normal_exit(self):
        process = self.child("import sys; sys.stdout.write('output'); sys.stderr.write('diagnostic')")
        capture = BoundedCapture(process)
        try:
            capture.finish()
            self.assertEqual(bytes(capture.captured["stdout"]), b"output")
            self.assertEqual(bytes(capture.captured["stderr"]), b"diagnostic")
        finally:
            capture.stop()

    def test_excess_native_output_is_killed_and_retention_stays_bounded(self):
        process = self.child("import os; os.write(1, b'x' * 262144)")
        capture = BoundedCapture(process)
        try:
            with self.assertRaises(ProbeError):
                capture.finish()
            self.assertLessEqual(sum(len(value) for value in capture.captured.values()), MAX_OUTPUT_BYTES)
            self.assertTrue(capture.overflow.is_set())
        finally:
            capture.stop()

    def test_concurrent_stdout_stderr_overflow_dispatches_once_before_reaping(self):
        source = """import os, threading
def write(fd):
    for _ in range(256):
        os.write(fd, b'x' * 4096)
threads = [threading.Thread(target=write, args=(fd,)) for fd in (1, 2)]
for thread in threads: thread.start()
for thread in threads: thread.join()
"""
        process = self.child(source)
        calls, thread_errors = [], []
        native_killpg = os.killpg

        def dispatch(pgid, number):
            calls.append((pgid, number, process.returncode))
            return native_killpg(pgid, number)

        with patch("run_host_probe.os.killpg", side_effect=dispatch), \
             patch.object(threading, "excepthook", side_effect=thread_errors.append):
            capture = BoundedCapture(process)
            try:
                with self.assertRaises(ProbeError):
                    capture.finish()
                # The real child/pipe boundary distinguishes one retained
                # dispatch from repeated kills of buffered bytes after wait.
                self.assertEqual(calls, [(process.pid, signal.SIGKILL, None)])
                self.assertFalse(thread_errors)
                self.assertTrue(capture.overflow.is_set())
                self.assertLessEqual(sum(map(len, capture.captured.values())), MAX_OUTPUT_BYTES)
                self.assertTrue(all(not reader.is_alive() for reader in capture.readers))
                self.assertIsNotNone(process.returncode)
                capture.stop()
                self.assertEqual(len(calls), 1)
            finally:
                capture.stop()

    def test_group_signal_denial_is_sticky_and_never_escapes_reader(self):
        process = self.child("import os; os.write(1, b'x' * 262144)")
        errors = []
        with patch("run_host_probe.os.killpg", side_effect=PermissionError()), \
             patch.object(threading, "excepthook", side_effect=errors.append):
            capture = BoundedCapture(process)
            try:
                with self.assertRaisesRegex(ProbeError, "cleanup remains unknown"):
                    capture.finish()
                self.assertFalse(errors)
                self.assertIsNotNone(process.returncode)
                self.assertTrue(capture.overflow.is_set())
                self.assertLessEqual(sum(map(len, capture.captured.values())), MAX_OUTPUT_BYTES)
                with self.assertRaisesRegex(ProbeError, "cleanup remains unknown"):
                    capture.stop()
            finally:
                with self.assertRaises(ProbeError):
                    capture.stop()

    def test_open_pipe_deadline_stays_unknown_after_child_exits_cleanly(self):
        # The parent intentionally retains a writer. Exact direct-child exit
        # cannot turn that incomplete output teardown into a successful receipt.
        read_fd, write_fd = os.pipe()
        process = subprocess.Popen([sys.executable, "-c", "print('output')"],
            stdin=subprocess.DEVNULL, stdout=write_fd, stderr=subprocess.PIPE,
            start_new_session=True)
        process.stdout = os.fdopen(read_fd, "rb")
        capture = BoundedCapture(process)
        try:
            with self.assertRaisesRegex(ProbeError, "cleanup remains unknown"):
                capture.join(timeout=0.05)
            os.close(write_fd)
            write_fd = None
            with self.assertRaisesRegex(ProbeError, "cleanup remains unknown"):
                capture.finish()
            self.assertEqual(process.returncode, 0)
            self.assertTrue(all(not reader.is_alive() for reader in capture.readers))
        finally:
            if write_fd is not None:
                os.close(write_fd)
            with self.assertRaises(ProbeError):
                capture.stop()


if __name__ == "__main__":
    unittest.main()
