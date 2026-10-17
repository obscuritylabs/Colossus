"""Portable exact-byte and hostile-chunk checks for the native screenshot acceptance client."""
import base64
import hashlib
import random
import struct
import unittest
import zlib

from host_capture_probe import MAX_CHUNK, ProbeError, capture, verified_png


def png(width=1280, height=800, dense=False):
    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))
    if dense:
        pixels = random.Random(7).randbytes(width * height * 3)
        rows = b"".join(b"\0" + pixels[offset:offset + width * 3] for offset in range(0, len(pixels), width * 3))
    else:
        rows = (b"\0" + b"\x12\x34\x56" * width) * height
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


class Channel:
    def __init__(self, content, command, failure=None):
        self.content, self.command, self.failure = content, command, failure
        self.offset, self.chunks = 0, 0

    def request(self, value):
        if value["operation"] == "capture":
            descriptor = {key: self.command[key] for key in ("session_id", "target", "control_generation")}
            descriptor.update(transfer_id="a" * 32, size_bytes=len(self.content), sha256=hashlib.sha256(self.content).hexdigest(), width=1280, height=800)
            if self.failure == "digest":
                descriptor["sha256"] = "0" * 64
            return {"result": "captured", "descriptor": descriptor}
        if self.offset == len(self.content):
            return {"result": "rejected", "code": "stale"}
        if value["request"]["offset"] != self.offset:
            raise AssertionError("acceptance client nominated an unordered offset")
        content = self.content[self.offset:self.offset + MAX_CHUNK]
        chunk = {"offset": self.offset, "data_base64": base64.b64encode(content).decode()}
        self.offset += len(content)
        self.chunks += 1
        if self.failure == "offset":
            chunk["offset"] += 1
        elif self.failure == "oversized":
            chunk["data_base64"] = "A" * (((MAX_CHUNK + 2) // 3) * 4 + 4)
        elif self.failure == "encoding":
            chunk["data_base64"] = "?"
        return {"result": "screenshot_chunk", "chunk": chunk}


class CaptureReceiptTests(unittest.TestCase):
    def command(self):
        return {"binding": {"application_id": "synthetic-fixture"}, "run_id": "run", "session_id": "session", "target": {"tab_id": "tab", "document_id": "document"}, "control_generation": 1}

    def test_real_png_crosses_multiple_bounded_private_chunks(self):
        command = self.command()
        channel = Channel(png(dense=True), command)
        receipt = capture(channel, command)
        self.assertGreater(channel.chunks, 1)
        self.assertEqual(set(receipt.values()), {True})
        self.assertNotIn("transfer_id", receipt)

    def test_bad_offset_hash_encoding_and_oversize_never_acknowledge(self):
        for failure in ("offset", "digest", "encoding", "oversized"):
            command = self.command()
            with self.subTest(failure=failure), self.assertRaises(ProbeError):
                capture(Channel(png(), command, failure), command)

    def test_crc_truncation_wrong_viewport_and_missing_pixels_fail_closed(self):
        valid = png()
        corrupt = bytearray(valid)
        corrupt[-5] ^= 1
        for content in (bytes(corrupt), valid[:-4], png(1, 1), valid[:33]):
            with self.assertRaises(ProbeError):
                verified_png(content)


if __name__ == "__main__":
    unittest.main()
