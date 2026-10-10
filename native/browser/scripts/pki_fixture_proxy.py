"""Bounded authenticated CONNECT relay to one disposable TLS fixture only."""

from __future__ import annotations

import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
import select
import socket
import threading
import time
from urllib.parse import urlsplit

from pki_fixture import RequestReader


class FixtureProxy:
    """Never forwards arbitrary HTTP, DNS destinations, or user-supplied ports."""

    def __init__(self, metadata: dict):
        self.password = os.urandom(32).hex()
        self.authorities = {f"127.0.0.1:{urlsplit(value).port}"
                            for value in metadata["urls"].values()}
        authorization = "Basic " + base64.b64encode(
            ("colossus:" + self.password).encode("ascii")).decode("ascii")
        self.connections, self.workers = set(), set()
        self.lock = threading.Lock()
        self.capacity = threading.BoundedSemaphore(16)
        self.requests = []
        self.overflow = False
        self.closing = False
        proxy = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def setup(self):
                super().setup()
                self.rfile = RequestReader(self.rfile, self.connection)

            def log_message(self, *_):
                pass

            def do_CONNECT(self):
                if self.headers.get_all("Proxy-Authorization", []) != [authorization]:
                    self.send_response(407)
                    self.send_header("Proxy-Authenticate", 'Basic realm="colossus-pki-fixture"')
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if self.path not in proxy.authorities or self.headers.get("Transfer-Encoding"):
                    self.send_error(403)
                    return
                port = int(self.path.removeprefix("127.0.0.1:"))
                upstream = socket.create_connection(("127.0.0.1", port), timeout=2)
                try:
                    with proxy.lock:
                        if proxy.closing:
                            return
                        proxy.connections.add(upstream)
                        if len(proxy.requests) >= 512:
                            proxy.overflow = True
                            return
                        proxy.requests.append(self.path)
                    self.send_response(200, "Connection established")
                    self.close_connection = True
                    self.end_headers()
                    self.wfile.flush()
                    peers = {self.connection: upstream, upstream: self.connection}
                    deadline, remaining = time.monotonic() + 15, 16 * 1024 * 1024
                    for stream in peers:
                        stream.settimeout(1)
                    while time.monotonic() < deadline and remaining > 0:
                        readable, _, _ = select.select(list(peers), [], [],
                                                       min(1, deadline - time.monotonic()))
                        for stream in readable:
                            data = stream.recv(min(65536, remaining))
                            if not data:
                                return
                            remaining -= len(data)
                            peers[stream].sendall(data)
                finally:
                    with proxy.lock:
                        proxy.connections.discard(upstream)
                    upstream.close()

        class Server(ThreadingHTTPServer):
            daemon_threads = True
            block_on_close = False
            request_queue_size = 16

            def process_request(self, request, address):
                if not proxy.capacity.acquire(blocking=False):
                    self.shutdown_request(request)
                    return
                with proxy.lock:
                    if proxy.closing:
                        proxy.capacity.release()
                        self.shutdown_request(request)
                        return
                    proxy.connections.add(request)
                try:
                    super().process_request(request, address)
                except BaseException:
                    with proxy.lock:
                        proxy.connections.discard(request)
                    proxy.capacity.release()
                    self.shutdown_request(request)
                    raise

            def process_request_thread(self, request, address):
                worker = threading.current_thread()
                with proxy.lock:
                    proxy.workers.add(worker)
                try:
                    super().process_request_thread(request, address)
                finally:
                    with proxy.lock:
                        proxy.connections.discard(request)
                        proxy.workers.discard(worker)
                    proxy.capacity.release()

            def handle_error(self, *_):
                pass

        self.server = Server(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever,
            kwargs={"poll_interval": 0.05}, daemon=True)
        self.thread.start()

    def configuration(self) -> dict:
        return {"address": "127.0.0.1", "port": self.server.server_address[1],
                "username": "colossus", "password": self.password}

    def close(self) -> None:
        with self.lock:
            self.closing = True
            for stream in self.connections:
                try:
                    stream.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                stream.close()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
        with self.lock:
            workers = list(self.workers)
        for worker in workers:
            worker.join(timeout=3)
        if self.thread.is_alive() or any(worker.is_alive() for worker in workers):
            raise ValueError("PKI fixture proxy did not settle")
