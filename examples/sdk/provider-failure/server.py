"""OpenAI-compatible fixture that deterministically rejects generation with HTTP 503."""

from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

_HOST = "127.0.0.1"
_PORT = 8100
_MODEL = "sdk-failure-model"


class Handler(BaseHTTPRequestHandler):
    """Advertise one model and return a bounded retryable generation failure."""

    server_version = "ColossusSDKFailureFixture/1"
    sys_version = ""

    def do_GET(self) -> None:  # noqa: N802
        if urlsplit(self.path).path != "/v1/models":
            self._json(404, {"error": {"message": "not found"}})
            return
        self._json(
            200,
            {
                "object": "list",
                "data": [{"id": _MODEL, "object": "model", "owned_by": "fixture"}],
            },
        )

    def do_POST(self) -> None:  # noqa: N802
        if urlsplit(self.path).path != "/v1/chat/completions":
            self._json(404, {"error": {"message": "not found"}})
            return
        content_length = int(self.headers.get("Content-Length", "0"))
        if content_length > 1_048_576:
            self._json(413, {"error": {"message": "request too large"}})
            return
        self.rfile.read(content_length)
        self._json(
            503,
            {"error": {"message": "fixture provider is warming up"}},
            retry_after="2",
        )

    def log_message(self, _format: str, *_arguments: object) -> None:
        return

    def _json(self, status: int, value: object, *, retry_after: str | None = None) -> None:
        body = json.dumps(value, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        if retry_after is not None:
            self.send_header("Retry-After", retry_after)
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    print(f"SDK failure fixture listening on http://{_HOST}:{_PORT}", flush=True)
    server = ThreadingHTTPServer((_HOST, _PORT), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
