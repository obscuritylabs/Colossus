#!/usr/bin/env python3
"""Bounded disposable HTTPS/mTLS fixtures; never install certificates or keys.

Successful fixture self-tests are not native Chromium or OS-store acceptance.
Native runners must additionally verify the observed peer certificate and retain
the separate signing, certificate custody/removal, and containment requirements.
"""

from __future__ import annotations

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import queue
import socket
import ssl
import sys
import threading
import time

from pki_fixture_material import generate
from pki_fixture_private import private_write


class RequestReader:
    """One absolute three-second/16 KiB HTTP-header budget, including slow input."""

    def __init__(self, source, connection):
        self.source, self.connection = source, connection
        self.deadline, self.remaining = time.monotonic() + 3, 16 * 1024

    def readline(self, limit=-1):
        remaining_time = self.deadline - time.monotonic()
        if remaining_time <= 0:
            raise TimeoutError("fixture request deadline expired")
        self.connection.settimeout(remaining_time)
        line = self.source.readline(min(limit if limit > 0 else 4097, 4097))
        self.remaining -= len(line)
        if self.remaining < 0 or (len(line) > 4096 and not line.endswith(b"\n")):
            raise ValueError("fixture request headers exceeded bounds")
        return line

    def close(self):
        self.source.close()


class Fixture:
    """Private unique material and finite loopback-only listeners."""

    def __init__(self, directory: Path, *, renderer_context: bool = False):
        self.directory = directory.absolute()
        # Closed native-custody fixture only: a title-only page may never create
        # a V8 context. Ordinary TLS fixtures keep their existing response bytes.
        self._renderer_context = renderer_context
        self.metadata = generate(self.directory)
        self._lock = threading.Lock()
        self._events = []
        self._overflow = False
        self._servers = []
        self._threads = []
        self._capacity = threading.BoundedSemaphore(16)
        self._closing = threading.Lock()
        self.metadata["urls"] = {}

    def event(self, endpoint: str, path: str, fingerprint: str | None, status: int) -> None:
        with self._lock:
            if len(self._events) >= 512:
                self._overflow = True
                return
            self._events.append({"endpoint": endpoint, "path": path,
                "client_fingerprint_sha256": fingerprint, "status": status})

    def report(self) -> dict:
        with self._lock:
            return {"schema_version": 1, "fixture_id": self.metadata["fixture_id"],
                    "events": list(self._events), "overflow": self._overflow,
                    "native_browser_verified": False, "os_store_cleanup_verified": False}

    def start(self) -> dict:
        fixture = self
        for endpoint, certificate, mutual in (("public", "server", False),
                ("mtls", "server", True), ("redirect_mtls", "server", True),
                ("untrusted", "untrusted_server", False),
                ("wrong_hostname", "wrong_host_server", False),
                ("expired", "expired_server", False)):
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = ssl.TLSVersion.TLSv1_2
            context.load_cert_chain(self.directory / f"{certificate}.pem",
                                    self.directory / f"{certificate}.key")
            if mutual:
                context.verify_mode = ssl.CERT_REQUIRED
                context.load_verify_locations(cafile=self.directory / "ca.pem")

            class Handler(BaseHTTPRequestHandler):
                protocol_version = "HTTP/1.1"

                def handle_one_request(self):
                    # Bound the HTTP line before the inherited header parser's
                    # independent per-line/100-header bounds can allocate input.
                    self.rfile = RequestReader(self.rfile, self.connection)
                    self.raw_requestline = self.rfile.readline(4097)
                    if len(self.raw_requestline) > 4096:
                        self.requestline = ""
                        self.request_version = ""
                        self.command = ""
                        self.send_error(414)
                        self.close_connection = True
                        return
                    if not self.raw_requestline:
                        self.close_connection = True
                        return
                    if not self.parse_request():
                        return
                    if self.command != "GET":
                        self.send_error(405)
                        self.close_connection = True
                        return
                    self.do_GET()
                    self.wfile.flush()

                def log_message(self, *_):
                    pass

                def do_GET(self):
                    path = self.path if self.path in ("/public", "/client-check", "/redirect-denied") else "unknown"
                    peer = self.connection.getpeercert(binary_form=True)
                    fingerprint = hashlib.sha256(peer).hexdigest() if peer else None
                    endpoint_name = self.server.endpoint
                    if path == "/redirect-denied" and endpoint_name in ("public", "mtls"):
                        status, body = 302, b""
                    elif path == "/client-check" and endpoint_name in ("mtls", "redirect_mtls"):
                        expected = fixture.metadata["fingerprints_sha256"]["client"]
                        status = 200 if fingerprint == expected else 403
                        body = b"<!doctype html><title>Colossus mTLS identity verified</title>" if status == 200 else b"<!doctype html><title>Colossus identity rejected</title>"
                        if fixture._renderer_context and endpoint_name == "mtls" and status == 200:
                            body += b"<script>void 0;</script>"
                    elif path == "/public":
                        status, body = 200, b"<!doctype html><title>Colossus private CA verified</title>"
                    else:
                        status, body = 404, b"fixture route unavailable"
                    fixture.event(endpoint_name, path, fingerprint, status)
                    self.send_response(status)
                    if status == 302:
                        self.send_header("Location", fixture.metadata["urls"]["redirect_mtls"] + "/client-check")
                    self.send_header("Content-Type", "text/html; charset=utf-8")
                    self.send_header("Cache-Control", "no-store")
                    self.send_header("Connection", "close")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    self.close_connection = True

            class Server(ThreadingHTTPServer):
                daemon_threads = True
                block_on_close = False
                request_queue_size = 16

                def get_request(self):
                    connection, address = super().get_request()
                    connection.settimeout(2)
                    with self.active_lock:
                        if self.closing:
                            connection.close()
                            raise OSError("fixture closing")
                        self.active.add(connection)
                    try:
                        secured = self.tls.wrap_socket(connection, server_side=True)
                        with self.active_lock:
                            self.active.discard(connection)
                            if self.closing:
                                secured.close()
                                raise OSError("fixture closing")
                            self.active.add(secured)
                        return secured, address
                    except (ssl.SSLError, OSError):
                        fixture.event(self.endpoint, "tls_denied", None, 0)
                        with self.active_lock:
                            self.active.discard(connection)
                        connection.close()
                        raise OSError("fixture TLS handshake denied") from None

                def shutdown_request(self, request):
                    try:
                        super().shutdown_request(request)
                    finally:
                        with self.active_lock:
                            self.active.discard(request)

                def process_request(self, request, address):
                    if not fixture._capacity.acquire(blocking=False):
                        self.shutdown_request(request)
                        fixture.event(self.endpoint, "capacity_denied", None, 0)
                        return
                    try:
                        super().process_request(request, address)
                    except BaseException:
                        fixture._capacity.release()
                        self.shutdown_request(request)
                        raise

                def process_request_thread(self, request, address):
                    try:
                        super().process_request_thread(request, address)
                    finally:
                        fixture._capacity.release()

                def handle_error(self, *_):
                    # No client-controlled TLS/parser diagnostics reach logs.
                    fixture.event(self.endpoint, "request_failed", None, 0)

            server = Server(("127.0.0.1", 0), Handler)
            server.endpoint, server.tls = endpoint, context
            server.active_lock, server.active, server.closing = threading.Lock(), set(), False
            self._servers.append(server)
            self.metadata["urls"][endpoint] = f"https://127.0.0.1:{server.server_address[1]}"
        # Populate every redirect target before any fixture accepts a request.
        for server in self._servers:
            thread = threading.Thread(target=server.serve_forever,
                                      kwargs={"poll_interval": 0.05}, daemon=True)
            thread.start()
            self._threads.append(thread)
        private_write(self.directory / "fixture.json",
                      (json.dumps(self.metadata, indent=2) + "\n").encode("utf-8"))
        return self.metadata

    def close(self) -> None:
        with self._closing:
            for index, server in enumerate(self._servers):
                with server.active_lock:
                    server.closing = True
                    for connection in server.active:
                        try:
                            connection.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            pass
                        connection.close()
                if index < len(self._threads) and self._threads[index].is_alive():
                    server.shutdown()
                server.server_close()
            for thread in self._threads:
                thread.join(timeout=3)
            if any(thread.is_alive() for thread in self._threads):
                raise ValueError("fixture listeners did not settle")
            self._servers.clear()
            self._threads.clear()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True,
                        help="New private directory; existing directories are rejected")
    arguments = parser.parse_args()
    fixture = Fixture(arguments.directory)
    try:
        metadata = fixture.start()
        print(json.dumps({"ready": True, "directory": str(fixture.directory), "fixture": metadata}), flush=True)
        messages = queue.Queue(maxsize=8)

        def read_messages():
            while True:
                line = sys.stdin.readline(65)
                messages.put(line)
                if not line or len(line) > 64:
                    return

        threading.Thread(target=read_messages, daemon=True).start()
        # Independent wall-clock lifetime, including a stalled operator or a
        # still-open control pipe after its native child died.
        deadline = time.monotonic() + 3600
        while True:
            try:
                line = messages.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty:
                raise ValueError("disposable PKI fixture lifetime expired") from None
            if line == "report\n":
                print(json.dumps({"report": fixture.report()}), flush=True)
            elif line in ("stop\n", ""):
                break
            else:
                raise ValueError("fixture accepts only report or stop control messages")
    finally:
        fixture.close()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError) as error:
        print(f"Disposable PKI fixture failed: {error}", file=sys.stderr)
        raise SystemExit(1) from None
