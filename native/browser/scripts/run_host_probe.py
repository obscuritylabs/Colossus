#!/usr/bin/env python3
"""Developer-only acceptance of the authenticated Linux CEF host and typed actions.

This fixture does not grant release capabilities or prove OS egress containment.
It passes only synthetic public page data through an authenticated private bridge.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import signal
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

import component
from run_probe import MAX_OUTPUT_BYTES, ProbeError, stop_process

FIXTURE = Path(__file__).resolve().parent.parent / "tests" / "host_fixture.html"
ORIGIN = "http://colossus-host-fixture.invalid"
MAX_FRAME = 128 * 1024


def encoded(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def exact(stream: socket.socket, size: int) -> bytes:
    result = bytearray()
    while len(result) < size:
        block = stream.recv(size - len(result))
        if not block:
            raise ProbeError("native host closed its private channel before acknowledgement")
        result.extend(block)
    return bytes(result)


def startup_receipt(stream: socket.socket) -> None:
    """Verify finite native initialization before requesting bridge readiness."""
    for phase in range(1, 8):
        try:
            receipt = exact(stream, 5)
        except (OSError, ProbeError) as error:
            raise ProbeError("native host disconnected after startup phase " + str(phase - 1)) from error
        if receipt[:4] != b"CBH\x01":
            raise ProbeError("native host returned an invalid startup receipt")
        if receipt[4] == (0x80 | (phase - 1)):
            raise ProbeError("native host rejected startup after phase " + str(phase - 1))
        if receipt[4] != phase:
            raise ProbeError("native host returned an out-of-order startup receipt")


class Channel:
    def __init__(self, stream: socket.socket, key: bytes, enrollment: dict, role: bytes):
        self.stream, self.key, self.role = stream, key, role
        self.digest = hashlib.sha256(encoded(enrollment)).digest()
        self.sequence = 0

    def tag(self, direction: bytes, sequence: int, payload: bytes) -> bytes:
        material = b"colossus-browser-bridge-v1\0" + self.digest + self.role + direction
        material += struct.pack(">QI", sequence, len(payload)) + payload
        return hmac.digest(self.key, material, "sha256")

    def request(self, value: dict) -> dict:
        self.sequence += 1
        payload = encoded(value)
        if not 0 < len(payload) <= MAX_FRAME:
            raise ProbeError("native fixture request exceeds private frame bounds")
        self.stream.sendall(struct.pack(">IQ", len(payload), self.sequence) + self.tag(b"request", self.sequence, payload) + payload)
        length, sequence = struct.unpack(">IQ", exact(self.stream, 12))
        if not 0 < length <= MAX_FRAME or sequence != self.sequence:
            raise ProbeError("native response violates private frame bounds or sequence")
        tag, payload = exact(self.stream, 32), exact(self.stream, length)
        if not hmac.compare_digest(tag, self.tag(b"response", sequence, payload)):
            raise ProbeError("native response authentication failed")
        value = json.loads(payload)
        if not isinstance(value, dict):
            raise ProbeError("native response is not a typed object")
        return value


def identity(prefix: str) -> str:
    return prefix + "_" + os.urandom(16).hex()


class BoundedCapture:
    """Drain both native streams continuously without retaining unbounded logs."""
    def __init__(self, process: subprocess.Popen):
        self.process = process
        self.captured = {"stdout":bytearray(), "stderr":bytearray()}
        self.lock = threading.Lock()
        self.overflow = threading.Event()
        self.readers = [threading.Thread(target=self.drain, args=(name, stream), daemon=True)
                        for name, stream in (("stdout",process.stdout),("stderr",process.stderr))]
        for reader in self.readers:
            reader.start()

    def drain(self, name, stream):
        try:
            while block := stream.read(4096):
                with self.lock:
                    available = MAX_OUTPUT_BYTES - sum(len(value) for value in self.captured.values())
                    self.captured[name].extend(block[:available])
                    if len(block) > available:
                        self.overflow.set()
                if self.overflow.is_set():
                    try:
                        os.killpg(self.process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
        finally:
            stream.close()

    def finish(self):
        self.process.wait(timeout=10)
        self.join()
        if self.process.returncode != 0 or self.overflow.is_set():
            raise ProbeError("native host failed its bounded acknowledged shutdown")
        deadline = time.monotonic() + 1
        while True:
            try:
                os.killpg(self.process.pid, 0)
            except ProcessLookupError:
                break
            if time.monotonic() >= deadline:
                raise ProbeError("native CEF shutdown left an owned helper process alive")
            time.sleep(0.01)

    def join(self):
        for reader in self.readers:
            reader.join(timeout=1)
        if any(reader.is_alive() for reader in self.readers):
            raise ProbeError("native host left an output channel open after termination")


def run_host_probe(root: Path, host: Path, screenshot: bool = False, transfers: bool = False) -> dict:
    if sys.platform != "linux":
        raise ProbeError("the standalone native host acceptance currently requires Linux")
    root = component.directory(root)
    inventory = component.verify_installed(root)
    manifest_digest = component.digest_file(root / component.MANIFEST)
    if inventory["platform"] != "linux64":
        raise ProbeError("native host needs the pinned Linux component")
    host = host.resolve(strict=True)
    if host.name != "colossus-native-browser-host" or not host.is_file() or not os.access(host, os.X_OK):
        raise ProbeError("supply the separately built native host executable")
    host_digest = component.digest_file(host)
    if FIXTURE.is_symlink() or FIXTURE.stat().st_size > 1024 * 1024:
        raise ProbeError("unsafe native host fixture")
    page = FIXTURE.read_bytes()
    if transfers:
        from host_transfer_probe import fixture as transfer_page
        page = transfer_page(page)
    credential = os.urandom(32).hex()
    expected_authorization = "Basic " + base64.b64encode(("colossus:" + credential).encode()).decode()
    requests = []
    uploaded = []

    class Proxy(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        def do_GET(self):
            if self.headers.get("Proxy-Authorization") != expected_authorization:
                self.send_response(407)
                self.send_header("Proxy-Authenticate", 'Basic realm="colossus-fixture"')
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if transfers:
                from host_transfer_probe import serve_get
                if serve_get(self, ORIGIN):
                    requests.append(self.path)
                    return
            if self.path not in (ORIGIN + "/fixture.html", ORIGIN + "/second.html"):
                self.send_error(403)
                return
            requests.append(self.path)
            content = page if self.path.endswith("/fixture.html") else page.replace(
                b"Colossus authenticated host fixture", b"Colossus second host document")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content)

        def do_POST(self):
            if not transfers or self.headers.get("Proxy-Authorization") != expected_authorization or self.path != ORIGIN + "/upload":
                self.send_error(403)
                return
            from host_transfer_probe import serve_upload
            serve_upload(self, page, uploaded)
            requests.append(self.path)

        def log_message(self, *_):
            pass

    proxy = ThreadingHTTPServer(("127.0.0.1", 0), Proxy)
    proxy.daemon_threads = True
    serving = threading.Thread(target=proxy.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
    serving.start()
    temporary = Path(tempfile.mkdtemp(prefix="colossus-host-fixture-"))
    process = None
    capture = None
    channels = []
    try:
        profile = temporary / "profile"
        profile.mkdir(mode=0o700)
        pairs = [socket.socketpair() for _ in range(3)]
        channels = [stream for pair in pairs for stream in pair]
        for parent, child in pairs:
            parent.settimeout(30)
            # Python timeout mode changes the descriptor's O_NONBLOCK flag.
            # The native bootstrap consumes exact bytes synchronously before
            # adopting its data/control channels into the async runtime.
            child.setblocking(True)
        key = os.urandom(32)
        binding = {"runtime_id":"native-fixture-runtime", "workspace_id":"native-fixture-workspace", "application_id":"native-fixture-app", "scope":{"kind":"conversation", "id":"native-fixture-conversation"}}
        actions = ["navigate", "snapshot", "click", "fill", "select", "press", "scroll", "wait", "back", "forward", "reload", "stop", "tab_open", "tab_select", "tab_close"]
        if screenshot:
            actions.append("screenshot")
        if transfers:
            actions.extend(("upload", "download"))
        # These flags exercise the trusted transport contract only in this
        # synthetic fixture. They never mutate the installed component modes.
        capabilities = {"available":True, "engine_version":component.CHROMIUM_VERSION, "modes":["headless"], "actions":actions, "limits":{"max_sessions":1,"max_tabs":8,"max_concurrent_actions":1,"max_snapshot_nodes":1024,"max_observation_bytes":65536,"action_timeout_ms":30000,"navigation_timeout_ms":60000,"max_lease_ms":1800000}, "private_ca_trust":False,"client_identities":False,"restrictive_egress":True}
        enrollment = {"binding":binding,"mode":"headless","profile":{"kind":"temporary"},"allowed_origins":[ORIGIN],"instance_nonce":list(os.urandom(16)),"component_digest":list(hashlib.sha256(bytes.fromhex(host_digest) + encoded(inventory)).digest()),"cancellation_closes_context":True,"capabilities":capabilities}
        configuration = encoded({"enrollment":enrollment,"profile_path":str(profile),"proxy":{"address":"127.0.0.1","port":proxy.server_address[1],"username":"colossus","password":credential}})
        child_streams = [pair[1] for pair in pairs]
        process = subprocess.Popen([str(host), *(str(stream.fileno()) for stream in child_streams)], cwd=temporary,
            env={"PATH":os.defpath,"HOME":str(temporary),"LANG":"C.UTF-8","LD_LIBRARY_PATH":str(root)},
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            pass_fds=tuple(stream.fileno() for stream in child_streams), start_new_session=True)
        capture = BoundedCapture(process)
        for stream in child_streams:
            stream.close()
        pairs[0][0].sendall(key + struct.pack(">I", len(configuration)) + configuration)
        startup_receipt(pairs[0][0])
        pairs[0][0].close()
        data = Channel(pairs[1][0], key, enrollment, b"data")
        control = Channel(pairs[2][0], key, enrollment, b"control")
        for channel in (data, control):
            if channel.request({"operation":"ready"}) != {"result":"ready"}:
                raise ProbeError("native host did not acknowledge authenticated readiness")
        session, tab, document = identity("bs"), identity("bt"), identity("bd")
        opened = data.request({"operation":"open","request":{"binding":binding,"run_id":"native-fixture-run","session_id":session,"tab_id":tab,"document_id":document,"options":{"mode":"headless","allowed_origins":[ORIGIN],"initial_url":ORIGIN + "/fixture.html"}}})
        if opened.get("result") != "opened" or opened.get("tab", {}).get("origin") != ORIGIN:
            raise ProbeError("native host did not allocate the exact fixture origin")

        documents = {tab: document}

        def action(value: dict, snapshot=None, new_tab=None):
            nonlocal document, tab
            target_tab = value["tab_id"] if value["kind"] in ("tab_select", "tab_close") else tab
            next_document = new_tab["document_id"] if new_tab else identity("bd")
            result = data.request({"operation":"execute","command":{"binding":binding,"run_id":"native-fixture-run","session_id":session,"target":{"tab_id":target_tab,"document_id":documents.get(target_tab, document)},"control_generation":1,"action":value,"next_document_id":next_document,"snapshot_id":snapshot,"new_tab":new_tab}})
            if result.get("result") == "observed":
                document = result["observation"]["tab"]["document_id"]
                tab = result["observation"]["tab"]["tab_id"]
                documents[tab] = document
            return result

        def observed(value: dict, *, new_tab=None):
            result = action(value, new_tab=new_tab)
            if result.get("result") != "observed":
                raise ProbeError("native host failed the bounded " + value["kind"] + " fixture")
            return result["observation"]

        def snapshot_contains(text: str):
            deadline = time.monotonic() + 5
            while True:
                result = action({"kind":"snapshot","max_nodes":128}, identity("bn"))
                if result.get("result") != "observed":
                    raise ProbeError("native host could not inspect the current action result")
                if text in encoded(result).decode():
                    return result
                if time.monotonic() >= deadline:
                    raise ProbeError("native action did not update its actual document")
                time.sleep(0.025)

        inspected = action({"kind":"snapshot","max_nodes":128}, identity("bn"))
        if inspected.get("result") != "observed":
            raise ProbeError("native host semantic snapshot failed")
        nodes = inspected["observation"]["snapshot"]["nodes"]
        if b"synthetic-protected-value" in encoded(inspected):
            raise ProbeError("native snapshot released a protected fixture value")
        button = next((node for node in nodes if node["role"] == "button" and node["name"] == "Continue"), None)
        ordinary = next((node for node in nodes if node["role"] == "textbox" and "Ordinary query" in node["name"]), None)
        protected = next((node for node in nodes if node["role"] == "textbox" and "Protected fixture" in node["name"]), None)
        if not button or not ordinary or not protected:
            raise ProbeError("native host did not issue the expected semantic references")
        if action({"kind":"fill","element":protected["element"],"text":"blocked"}) != {"result":"rejected","code":"denied"}:
            raise ProbeError("native host accepted input to a protected field")
        if action({"kind":"fill","element":ordinary["element"],"text":"Typed native fixture"}).get("result") != "observed":
            raise ProbeError("native host did not fill the ordinary field")
        fresh = action({"kind":"snapshot","max_nodes":128}, identity("bn"))
        if fresh.get("result") != "observed" or "Typed native fixture" not in encoded(fresh).decode():
            raise ProbeError("native fill did not update the actual DOM")
        observed({"kind":"press","key":"end"})
        observed({"kind":"press","key":"backspace"})
        fresh = snapshot_contains("Typed native fixtur")
        ordinary = next(node for node in fresh["observation"]["snapshot"]["nodes"]
                        if node["role"] == "textbox" and "Ordinary query" in node["name"])
        if ordinary.get("value") != "Typed native fixtur":
            raise ProbeError("native safe key did not change the actual focused field")
        choice = next((node for node in fresh["observation"]["snapshot"]["nodes"] if node["role"] == "combobox" and "Fixture choice" in node["name"]), None)
        if not choice or action({"kind":"select","element":choice["element"],"values":["second"]}).get("result") != "observed":
            raise ProbeError("native host did not prove its ordinary selection")
        fresh = action({"kind":"snapshot","max_nodes":128}, identity("bn"))
        button = next(node for node in fresh["observation"]["snapshot"]["nodes"] if node["role"] == "button" and node["name"] == "Continue")
        if action({"kind":"click","element":button["element"]}).get("result") != "observed":
            raise ProbeError("native host click failed")
        clicked = action({"kind":"snapshot","max_nodes":128}, identity("bn"))
        if "Native click received" not in encoded(clicked).decode():
            raise ProbeError("native click did not update the actual DOM")
        button = next(node for node in clicked["observation"]["snapshot"]["nodes"]
                      if node["role"] == "button" and node["name"] == "Continue")
        observed({"kind":"wait","condition":{"kind":"element_visible","element":button["element"]},"timeout_ms":1000})
        observed({"kind":"wait","condition":{"kind":"load"},"timeout_ms":1000})
        if action({"kind":"scroll","x":-2147483648,"y":0}) != {"result":"rejected","code":"limit_exceeded"}:
            raise ProbeError("native host accepted an overflowing scroll distance")
        observed({"kind":"scroll","x":0,"y":400})
        snapshot_contains("Native scroll received")
        observed({"kind":"navigate","url":ORIGIN + "/second.html"})
        snapshot_contains("Colossus second host document")
        observed({"kind":"back"})
        snapshot_contains("Colossus authenticated host fixture")
        observed({"kind":"forward"})
        snapshot_contains("Colossus second host document")
        observed({"kind":"reload"})
        snapshot_contains("Colossus second host document")
        observed({"kind":"stop"})
        original_tab = tab
        second_tab = identity("bt")
        opened_tab = observed({"kind":"tab_open","url":ORIGIN + "/fixture.html"}, new_tab={
            "tab_id":second_tab,"document_id":identity("bd"),"origin":None,"title":""})
        if opened_tab["tab"]["tab_id"] != second_tab:
            raise ProbeError("native host did not bind its allocated opaque tab")
        snapshot_contains("Colossus authenticated host fixture")
        selected_tab = observed({"kind":"tab_select","tab_id":original_tab})
        if selected_tab["tab"]["tab_id"] != original_tab:
            raise ProbeError("native host did not select its exact existing tab")
        snapshot_contains("Colossus second host document")
        # The close observation intentionally names the removed tab. Retain the
        # live selected target independently to prove it remains usable.
        retained_document = document
        observed({"kind":"tab_close","tab_id":second_tab})
        tab, document = original_tab, retained_document
        snapshot_contains("Colossus second host document")
        if action({"kind":"tab_select","tab_id":second_tab}) != {"result":"rejected","code":"stale"}:
            raise ProbeError("native host retained authority for a closed tab")
        if action({"kind":"navigate","url":"https://denied.invalid/"}) != {"result":"rejected","code":"denied"}:
            raise ProbeError("native host accepted a destination outside its enrollment")
        screenshot_receipt = {}
        if screenshot:
            from host_capture_probe import capture as capture_screenshot
            screenshot_receipt = capture_screenshot(data, {"binding": binding, "run_id": "native-fixture-run", "session_id": session,
                "target": {"tab_id": tab, "document_id": document}, "control_generation": 1,
                "action": {"kind": "screenshot", "max_bytes": 4 * 1024 * 1024},
                "next_document_id": identity("bd"), "snapshot_id": None, "new_tab": None})
        transfer_receipt = {}
        if transfers:
            from host_transfer_probe import transfer_fixture
            def transfer_command(value):
                return {"binding": binding, "run_id": "native-fixture-run", "session_id": session,
                    "target": {"tab_id": tab, "document_id": document}, "control_generation": 1,
                    "action": value, "next_document_id": identity("bd"), "snapshot_id": None, "new_tab": None}
            transfer_receipt = transfer_fixture(data, transfer_command, action,
                lambda: action({"kind": "snapshot", "max_nodes": 128}, identity("bn")), uploaded, ORIGIN)
        screenshot_receipt.update(transfer_receipt)
        if control.request({"operation":"close","session_id":session}) != {"result":"acknowledged"}:
            raise ProbeError("native host did not acknowledge native close and CEF shutdown")
        pairs[1][0].close()
        pairs[2][0].close()
        capture.finish()
        if not requests:
            raise ProbeError("native host did not authenticate to its fixed proxy")
        if component.verify_installed(root) != inventory or component.digest_file(host) != host_digest or component.digest_file(root / component.MANIFEST) != manifest_digest:
            raise ProbeError("native host changed its inventoried browser component")
        return {**screenshot_receipt,"host_sha256":host_digest,"component_manifest_sha256":manifest_digest,"authenticated_bridge":True,"native_snapshot":True,"native_fill":True,"native_select":True,"native_click":True,"native_press":True,"native_scroll":True,"native_wait":True,"native_navigation_history":True,"native_reload":True,"native_stop_idle":True,"native_owned_tabs":True,"protected_fields":True,"origin_denial":True,"proxy_authentication":True,"cef_shutdown":True,"production_containment":False}
    finally:
        try:
            for stream in channels:
                stream.close()
            if process is not None:
                stop_process(process)
            if capture is not None:
                capture.join()
            # Delete the positively created fixture directory only after the
            # child group and both native output channels have been drained.
            shutil.rmtree(temporary)
        finally:
            proxy.shutdown()
            proxy.server_close()
            serving.join(timeout=1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--host", type=Path, required=True)
    parser.add_argument("--screenshot", action="store_true", help="require actual bounded PNG capture and consumed-transfer revocation")
    parser.add_argument("--transfers", action="store_true", help="require actual reviewed-file upload and bounded native HTTP download receipts")
    arguments = parser.parse_args()
    try:
        result = run_host_probe(arguments.component, arguments.host, arguments.screenshot, arguments.transfers)
        print(json.dumps(result, sort_keys=True))
        return 0
    except ProbeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except (component.ComponentError, OSError, ValueError, subprocess.TimeoutExpired):
        print("native authenticated host acceptance failed; inspect the native host build and compatible sandbox lane", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
