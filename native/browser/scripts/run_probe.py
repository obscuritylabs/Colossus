#!/usr/bin/env python3
"""Run the developer-only Linux headless CEF fixture without a display server."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import threading
import time

import component


FIXTURE = Path(__file__).resolve().parent.parent / "tests" / "fixture.html"
MAX_OUTPUT_BYTES = 64 * 1024
MAX_FIXTURE_BYTES = 1024 * 1024
PROBE_TIMEOUT_SECONDS = 30
PROBE_EXECUTABLE = "colossus-browser-probe"


class ProbeError(component.ComponentError):
    """The native probe did not establish its developer acceptance evidence."""


@dataclass(frozen=True)
class ProbeResult:
    returncode: int
    stdout: str
    stderr: str


def fixture_server(fixture: Path) -> ThreadingHTTPServer:
    if fixture.is_symlink() or not fixture.is_file() or fixture.stat().st_size > MAX_FIXTURE_BYTES:
        raise ProbeError("missing, linked, or oversized native browser fixture")
    payload = fixture.read_bytes()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            # Serve exactly one tracked HTML file. No directory traversal,
            # repository files, directory listing, or arbitrary fixture roots.
            if self.path != "/fixture.html":
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    return server


def stop_process(process: subprocess.Popen) -> None:
    # The probe owns a fresh process group. Kill its CEF helpers too when a
    # deadline or output bound fails; do not leave a renderer behind.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=5)


def bounded_process(command: list[str], cwd: Path, *, timeout: float) -> ProbeResult:
    environment = os.environ.copy()
    environment.pop("DISPLAY", None)
    environment.pop("WAYLAND_DISPLAY", None)
    # The fixture owns this fresh directory; CEF helpers and Fontconfig must
    # not discover an ambient user's browser state or cache directories.
    environment.update({
        "HOME": str(cwd),
        "XDG_CONFIG_HOME": str(cwd / ".config"),
        "XDG_DATA_HOME": str(cwd / ".local/share"),
        "XDG_CACHE_HOME": str(cwd / ".cache"),
    })
    process = subprocess.Popen(
        command, cwd=cwd, env=environment, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
    )
    captured = {"stdout": bytearray(), "stderr": bytearray()}
    lock = threading.Lock()
    overflow = threading.Event()

    def drain(name, stream):
        try:
            while block := stream.read(4096):
                with lock:
                    available = MAX_OUTPUT_BYTES - sum(len(value) for value in captured.values())
                    captured[name].extend(block[:available])
                    if len(block) > available:
                        overflow.set()
        finally:
            stream.close()

    readers = [
        threading.Thread(target=drain, args=(name, stream), daemon=True)
        for name, stream in (("stdout", process.stdout), ("stderr", process.stderr))
    ]
    for reader in readers:
        reader.start()
    deadline = time.monotonic() + timeout
    failure = None
    try:
        while process.poll() is None:
            if overflow.is_set():
                failure = "native probe exceeded its combined 64 KiB output limit"
                break
            if time.monotonic() >= deadline:
                failure = "native probe exceeded its 30 second deadline"
                break
            try:
                process.wait(timeout=min(0.05, max(0.001, deadline - time.monotonic())))
            except subprocess.TimeoutExpired:
                continue
    finally:
        stop_process(process)
        for reader in readers:
            reader.join(timeout=1)
    if failure or overflow.is_set():
        raise ProbeError(failure or "native probe exceeded its combined 64 KiB output limit")
    if any(reader.is_alive() for reader in readers):
        raise ProbeError("native probe left an output pipe open after termination")
    return ProbeResult(
        returncode=process.returncode,
        stdout=captured["stdout"].decode("utf-8", errors="replace"),
        stderr=captured["stderr"].decode("utf-8", errors="replace"),
    )


def run_probe(root: Path, *, fixture: Path = FIXTURE, timeout: float = PROBE_TIMEOUT_SECONDS,
              manifest_path: Path | None = None) -> ProbeResult:
    if sys.platform != "linux":
        raise ProbeError("this initial no-display native probe requires a Linux host")
    root = component.directory(root)
    manifest = component.verify_installed(root, manifest_path)
    if manifest["platform"] != "linux64" or manifest["executable"] != PROBE_EXECUTABLE:
        raise ProbeError("the component does not inventory the Linux x64 native probe executable")
    executable = root / PROBE_EXECUTABLE
    server = fixture_server(fixture)
    serving = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
    serving.start()
    try:
        port = server.server_address[1]
        with tempfile.TemporaryDirectory(prefix="colossus-native-probe-") as session:
            result = bounded_process(
                [str(executable), f"--fixture-url=http://127.0.0.1:{port}/fixture.html"],
                Path(session), timeout=timeout,
            )
    finally:
        server.shutdown()
        server.server_close()
        serving.join(timeout=1)
    if result.returncode != 0:
        raise ProbeError(
            f"native browser probe failed (exit {result.returncode}). Chromium's sandbox remains required; "
            "some restricted Linux runners prohibit user namespaces. Use a compatible native test host.\n"
            + result.stdout + result.stderr
        )
    if (
        "native_fixture=passed" not in result.stdout
        or "native_negative_controls=passed" not in result.stdout
        or "native_offscreen_pixels=passed" not in result.stdout
        or "native_presentation_input=passed" not in result.stdout
        or "native_presentation_revocation=passed" not in result.stdout
        or re.search(r"^devtools_command=1 success=1 bounded_bytes=\d+$", result.stdout, re.MULTILINE) is None
    ):
        raise ProbeError("native probe exited without all fixture, negative-control, and DevTools success evidence")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", type=Path, required=True, help="explicit inventoried component directory")
    parser.add_argument("--manifest", type=Path, help="external developer consistency inventory for the fixture executable")
    arguments = parser.parse_args()
    try:
        result = run_probe(arguments.component, manifest_path=arguments.manifest)
        print(result.stdout, end="")
        if result.stderr:
            print(result.stderr, end="", file=sys.stderr)
        print("native_probe=passed headless_release_support=false desktop_release_support=false")
        return 0
    except (component.ComponentError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"native browser acceptance: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
