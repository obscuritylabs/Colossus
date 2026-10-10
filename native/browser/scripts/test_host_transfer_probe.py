"""The native fixture must verify actual multipart bytes and ignore page paths."""
import io
import unittest
from email.message import Message
import host_transfer_probe as probe
from run_probe import ProbeError


class Handler:
    def __init__(self, path="", body=b"", content_type=""):
        self.path, self.rfile, self.wfile = path, io.BytesIO(body), io.BytesIO()
        self.headers = Message()
        self.headers["Content-Length"] = str(len(body))
        self.headers["Content-Type"] = content_type
        self.response, self.sent = None, {}
    def send_response(self, status):
        self.response = status
    def send_header(self, name, value):
        self.sent[name] = value
    def end_headers(self):
        pass


def multipart(data, name="upload.txt"):
    return (b'--bound\r\nContent-Disposition: form-data; name="owned_file"; filename="' + name.encode()
            + b'"\r\nContent-Type: application/octet-stream\r\n\r\n' + data + b'\r\n--bound--\r\n')


class Transfers(unittest.TestCase):
    def test_actual_http_upload_requires_exact_bytes_and_safe_display_name(self):
        received = []
        handler = Handler(body=multipart(probe.UPLOAD), content_type="multipart/form-data; boundary=bound")
        probe.serve_upload(handler, b"synthetic page", received)
        self.assertEqual(received, [True])
        self.assertEqual(handler.response, 200)
        for data, name in ((b"substituted", "upload.txt"), (probe.UPLOAD, "../upload.txt")):
            handler = Handler(body=multipart(data, name), content_type="multipart/form-data; boundary=bound")
            with self.assertRaises(ProbeError):
                probe.serve_upload(handler, b"page", [])

    def test_bounded_native_download_bytes_include_untrusted_page_filename(self):
        handler = Handler("http://fixture.invalid/download-final")
        self.assertTrue(probe.serve_get(handler, "http://fixture.invalid"))
        self.assertEqual(handler.wfile.getvalue(), probe.DOWNLOAD)
        self.assertIn("../../", handler.sent["Content-Disposition"])
        handler = Handler("http://fixture.invalid/download-empty")
        self.assertTrue(probe.serve_get(handler, "http://fixture.invalid"))
        self.assertEqual(handler.sent["Content-Length"], "0")

    def test_only_fixed_reviewed_download_routes_are_served(self):
        handler = Handler("http://fixture.invalid/download-redirect")
        self.assertTrue(probe.serve_get(handler, "http://fixture.invalid"))
        self.assertEqual(handler.sent["Location"], "http://fixture.invalid/download-final")
        self.assertFalse(probe.serve_get(Handler("http://foreign.invalid/download-final"), "http://fixture.invalid"))


if __name__ == "__main__":
    unittest.main()
