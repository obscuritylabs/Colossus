"""Credential-free loopback fixture for the SDK OpenAPI example."""

from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlsplit

_HOST = "127.0.0.1"
_PORT = 8099
_PREFIX = "/v1/status/"


class Handler(BaseHTTPRequestHandler):
    """Serve one bounded status operation without logging request data."""

    server_version = "ColossusSDKFixture/1"
    sys_version = ""

    def do_GET(self) -> None:  # noqa: N802
        path = urlsplit(self.path)
        if not path.path.startswith(_PREFIX):
            self._json(404, {"error": "not_found"})
            return
        service = unquote(path.path.removeprefix(_PREFIX))
        if not service or len(service.encode("utf-8")) > 64 or "/" in service:
            self._json(400, {"error": "invalid_service"})
            return
        self._json(200, {"service": service, "status": "green"})

    def log_message(self, _format: str, *_arguments: object) -> None:
        return

    def _json(self, status: int, value: object) -> None:
        body = json.dumps(value, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    print(f"SDK integration fixture listening on http://{_HOST}:{_PORT}", flush=True)
    server = ThreadingHTTPServer((_HOST, _PORT), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
