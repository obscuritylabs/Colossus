"""Finite private screenshot transfer receipt; no PNG content is printed or published."""
import base64
import binascii
import hashlib
import struct
import zlib

from run_probe import ProbeError

MAX_CAPTURE = 4 * 1024 * 1024
MAX_CHUNK = 64 * 1024


def verified_png(data):
    if not 33 <= len(data) <= MAX_CAPTURE or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ProbeError("native screenshot is not a bounded PNG")
    offset, compressed, dimensions, finished = 8, bytearray(), None, False
    while offset < len(data):
        if offset + 12 > len(data):
            raise ProbeError("native PNG is truncated")
        size = struct.unpack(">I", data[offset:offset + 4])[0]
        end = offset + 12 + size
        if end > len(data):
            raise ProbeError("native PNG chunk exceeds its verified bytes")
        kind, payload = data[offset + 4:offset + 8], data[offset + 8:end - 4]
        if zlib.crc32(kind + payload) != struct.unpack(">I", data[end - 4:end])[0]:
            raise ProbeError("native PNG chunk checksum failed")
        if kind == b"IHDR":
            if dimensions is not None or offset != 8 or size != 13:
                raise ProbeError("native PNG header is invalid")
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", payload)
            if (width, height) != (1280, 800) or depth != 8 or color not in (2, 6) or (compression, filtering, interlace) != (0, 0, 0):
                raise ProbeError("native screenshot viewport or pixel format changed")
            dimensions = (width, height, 3 if color == 2 else 4)
        elif kind == b"IDAT":
            compressed.extend(payload)
        elif kind == b"IEND":
            if size != 0 or end != len(data):
                raise ProbeError("native PNG termination is invalid")
            finished = True
        offset = end
    if not finished or dimensions is None or not compressed:
        raise ProbeError("native PNG pixel content absent")
    width, height, channels = dimensions
    expected = (width * channels + 1) * height
    decoder = zlib.decompressobj()
    pixels = decoder.decompress(compressed, expected + 1)
    if len(pixels) != expected or not decoder.eof or decoder.unconsumed_tail or decoder.unused_data:
        raise ProbeError("native PNG decoded pixels exceeded the fixed viewport")
    return width, height


def capture(data_channel, command):
    result = data_channel.request({"operation": "capture", "command": command})
    if result.get("result") != "captured":
        raise ProbeError("native capture did not acknowledge its private PNG transfer")
    descriptor = result.get("descriptor", {})
    if descriptor.get("session_id") != command["session_id"] or descriptor.get("target") != command["target"] or descriptor.get("control_generation") != command["control_generation"]:
        raise ProbeError("native screenshot ownership receipt changed")
    total = descriptor.get("size_bytes")
    if type(total) is not int or not 33 <= total <= MAX_CAPTURE:
        raise ProbeError("native screenshot size receipt exceeded its ceiling")
    transfer = descriptor.get("transfer_id")
    if not isinstance(transfer, str) or len(transfer) != 32 or any(character not in "0123456789abcdef" for character in transfer):
        raise ProbeError("native screenshot transfer identity invalid")
    png = bytearray()
    request = {key: command[key] for key in ("binding", "run_id", "session_id", "target", "control_generation")}
    request.update(transfer_id=transfer, offset=0)
    while len(png) < total:
        request["offset"] = len(png)
        result = data_channel.request({"operation": "read_screenshot", "request": request})
        chunk = result.get("chunk", {})
        encoded = chunk.get("data_base64")
        if result.get("result") != "screenshot_chunk" or chunk.get("offset") != len(png) or not isinstance(encoded, str) or len(encoded) > ((MAX_CHUNK + 2) // 3) * 4:
            raise ProbeError("native screenshot chunk receipt is invalid")
        try:
            decoded = base64.b64decode(encoded, validate=True)
        except (binascii.Error, ValueError) as error:
            raise ProbeError("native screenshot chunk encoding is invalid") from error
        if not 0 < len(decoded) <= MAX_CHUNK or len(png) + len(decoded) > total or base64.b64encode(decoded).decode() != encoded:
            raise ProbeError("native screenshot chunk exceeded its ordered transfer")
        png.extend(decoded)
    if hashlib.sha256(png).hexdigest() != descriptor.get("sha256") or verified_png(png) != (descriptor.get("width"), descriptor.get("height")):
        raise ProbeError("native screenshot bytes do not match the captured PNG receipt")
    request["offset"] = 0
    if data_channel.request({"operation": "read_screenshot", "request": request}) != {"result": "rejected", "code": "stale"}:
        raise ProbeError("native screenshot transfer remained readable after consumption")
    return {"native_png_capture": True, "native_png_dimensions": True, "bounded_private_chunks": True, "consumed_transfer_revoked": True}
