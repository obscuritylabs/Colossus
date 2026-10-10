#!/usr/bin/env python3
"""Actual sandbox-preserving Linux Chromium PKI/key-use fixture conformance.

Runs in a compatible native CEF lane, without a display server. Each case has
fresh native HOME/NSS/profile ownership, authenticated inherited channels, and
a fixed fixture-only CONNECT relay. This does not grant production containment,
signing, or installed macOS/Windows certificate-custody acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid

import component
from pki_fixture import Fixture
from pki_fixture_private import private_directory, private_write
from pki_fixture_proxy import FixtureProxy
from pki_fixture_retirement import FixtureInputs, StartupError, startup_receipt
from run_host_probe import Channel, encoded, identity
from run_probe import MAX_OUTPUT_BYTES, ProbeError, stop_process


def utility(path: Path) -> dict:
    path = path.resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ProbeError("supply executable, pinned native NSS utilities")
    return {"path": str(path), "sha256": component.digest_file(path)}


class NativeOutput:
    """Drain native diagnostics without blocking or retaining unbounded content."""

    def __init__(self, process: subprocess.Popen, *, custody_test=False):
        self.process, self.bytes, self.overflow = process, 0, False
        self.category = None
        self.startup_diagnostic = None
        self.custody_test = custody_test
        self.custody_open_denied = None
        self.custody_diagnostics = []
        self.lock = threading.Lock()
        self.receipt_changed = threading.Condition(self.lock)
        self.threads = []
        for stream in (process.stdout, process.stderr):
            thread = threading.Thread(target=self.drain, args=(stream,), daemon=True)
            thread.start()
            self.threads.append(thread)

    def drain(self, stream):
        # Retain only a small line fragment while matching the host's fixed,
        # categorical error vocabulary. Native diagnostics, paths, URLs and
        # bootstrap material are never emitted or retained in evidence.
        categories = {
            b"verified browser component unavailable": "unavailable",
            b"browser operation unsupported": "unsupported",
            b"browser native boundary denied operation": "denied",
            b"browser target is stale": "stale",
            b"browser operation cancelled": "cancelled",
            b"browser response exceeded bounds": "limit_exceeded",
            b"browser certificate configuration required": "authentication_required",
            b"browser native host failed": "failed",
            b"browser outcome unknown": "outcome_unknown",
        }
        fragment = b""
        diagnostic_tail = b""
        # BufferedReader.read may withhold a short fixed receipt until 4096
        # bytes or EOF. One bounded underlying read preserves live evidence.
        read = getattr(stream, "read1", stream.read)
        try:
            while block := read(4096):
                # Match only fixed startup failures. The ELF loader prefixes
                # symbol errors with arbitrary installation paths; classify
                # the known suffix without retaining or reporting that path.
                diagnostic_window = diagnostic_tail + block
                for marker, category in (
                    (b"undefined symbol: close", "elf_close_symbol_missing"),
                    (b"Running as root without --no-sandbox is not supported", "cef_root_rejected"),
                    (b"No usable sandbox!", "cef_sandbox_unavailable"),
                    (b"Failed to move to new namespace:", "cef_namespace_denied"),
                ):
                    if marker in diagnostic_window:
                        self.startup_diagnostic = category
                diagnostic_tail = diagnostic_window[-128:]
                for line in (fragment + block).splitlines(keepends=True):
                    if line.endswith(b"\n"):
                        if category := categories.get(line.rstrip(b"\r\n")):
                            self.category = category
                        if self.custody_test:
                            receipt = line.rstrip(b"\r\n")
                            diagnostics = ("DOCUMENT_COMMITTED", "READY_BEFORE_COMMIT", "CHALLENGE_SENT",
                                "OPEN_SUCCEEDED", "OPEN_OTHER_ERROR", "PROC_UNAVAILABLE", "PROCESS_NOT_FOUND",
                                "PROCESS_AMBIGUOUS", "SANDBOX_UNVERIFIED", "EXE_UNVERIFIED", "PIDFD_UNAVAILABLE",
                                "PROCESS_UNSTABLE", "DOCUMENT_UNVERIFIED", "INODE_UNVERIFIED")
                            # stdout/stderr drain concurrently. Failure remains
                            # sticky and the combined fixed vocabulary bounded.
                            with self.receipt_changed:
                                if receipt == b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED":
                                    self.custody_open_denied = self.custody_open_denied is not False
                                    self.receipt_changed.notify_all()
                                elif receipt == b"COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_FAILED":
                                    self.custody_open_denied = False
                                    self.receipt_changed.notify_all()
                                for diagnostic in diagnostics:
                                    if receipt == ("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_" + diagnostic).encode() \
                                            and len(self.custody_diagnostics) < 16:
                                        self.custody_diagnostics.append(diagnostic.lower())
                        fragment = b""
                    else:
                        fragment = line if len(line) <= 128 else b""
                with self.lock:
                    self.bytes += len(block)
                    if self.bytes > MAX_OUTPUT_BYTES:
                        self.overflow = True
                        break
        finally:
            stream.close()

    def wait_custody_receipt(self, timeout: float = 5) -> bool | None:
        """Wait only for a decoded fixed native receipt while CEF stays alive."""
        if not 0 <= timeout <= 5:
            raise ValueError("native receipt deadline must be within five seconds")
        if not self.custody_test:
            return None
        deadline = time.monotonic() + timeout
        with self.receipt_changed:
            while self.custody_open_denied is None:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None
                self.receipt_changed.wait(remaining)
            return self.custody_open_denied

    def finish(self) -> None:
        self.process.wait(timeout=15)
        self.join_output()
        if self.process.returncode != 0:
            raise ProbeError("native PKI host did not acknowledge clean shutdown")
        deadline = time.monotonic() + 1
        while True:
            try:
                os.killpg(self.process.pid, 0)
            except ProcessLookupError:
                break
            if time.monotonic() >= deadline:
                raise ProbeError("native PKI shutdown left an owned helper process alive")
            time.sleep(0.01)

    def join_output(self) -> None:
        for thread in self.threads:
            thread.join(timeout=3)
        if self.overflow or any(thread.is_alive() for thread in self.threads):
            raise ProbeError("native PKI output exceeded its bounds or did not settle")

    def settle_failure(self) -> None:
        # Bootstrap close can precede the host's fixed categorical exit by a
        # scheduling tick. Settle it before classifying, retaining no raw output.
        try:
            self.process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            stop_process(self.process)
        for thread in self.threads:
            thread.join(timeout=0.25)


def run_case(case: dict, fixture: Fixture, proxy: FixtureProxy, root: Path,
             host: Path, tools: dict, parent: Path, inventory: dict) -> dict:
    origins = list(fixture.metadata["urls"].values())
    target = fixture.metadata["urls"][case["endpoint"]]
    pki = {**tools, "require_source_retirement": True,
        "ca_files": [str(fixture.directory / "ca.der")] if case.get("trust", True) else [],
        "identities": [], "bindings": []}
    if selected := case.get("binding"):
        pki["bindings"].append({"origin": fixture.metadata["urls"]["mtls"],
            "fingerprint_sha256": fixture.metadata["fingerprints_sha256"][selected]})
    process, streams, output = None, [], None
    live_custody_receipt = None
    process_group_reaped = False
    phase = "profile"
    before = len(fixture.report()["events"])
    before_tunnels = len(proxy.requests)
    with tempfile.TemporaryDirectory(prefix="host-case-", dir=parent) as allocation, \
            FixtureInputs(Path(allocation) / "inputs") as inputs:
        allocation = Path(allocation)
        profile = allocation / "profile"
        private_directory(profile)
        for index, name in enumerate(case.get("identities", [])):
            pki["identities"].append({
                "pfx_file": inputs.copy(fixture.directory / f"{name}.pfx", f"identity-{index}.pfx", 4 * 1024 * 1024),
                "passphrase_file": inputs.copy(fixture.directory / "passphrase.txt", f"password-{index}", 128)})
        pairs = [socket.socketpair() for _ in range(3)]
        streams = [stream for pair in pairs for stream in pair]
        # Python timeout mode sets O_NONBLOCK on the socket's file description.
        # Only parent endpoints use it. Native bootstrap requires blocking reads
        # with its own bounded SO_RCVTIMEO before the async channels are created.
        for parent_stream, child_stream in pairs:
            parent_stream.settimeout(90)
            child_stream.setblocking(True)
        key = os.urandom(32)
        binding = {"runtime_id": "native-pki-fixture-runtime", "workspace_id": "native-pki-fixture-workspace",
            "application_id": "native-pki-fixture-app", "scope": {"kind": "conversation", "id": "native-pki-fixture-conversation"}}
        capabilities = {"available": True, "engine_version": component.CHROMIUM_VERSION,
            "modes": ["headless"], "actions": ["navigate", "snapshot"],
            "limits": {"max_sessions": 1, "max_tabs": 1, "max_concurrent_actions": 1,
                "max_snapshot_nodes": 128, "max_observation_bytes": 65536,
                "action_timeout_ms": 30000, "navigation_timeout_ms": 60000, "max_lease_ms": 1800000},
            "private_ca_trust": bool(pki["ca_files"]), "client_identities": bool(pki["identities"]), "restrictive_egress": True}
        enrollment = {"binding": binding, "mode": "headless", "profile": {"kind": "temporary"}, "allowed_origins": origins,
            "instance_nonce": list(os.urandom(16)), "component_digest": list(hashlib.sha256(
                bytes.fromhex(component.digest_file(host)) + encoded(inventory)).digest()),
            "cancellation_closes_context": True, "capabilities": capabilities}
        bootstrap = {"enrollment": enrollment, "profile_path": str(profile),
                     "proxy": proxy.configuration(), "pki": pki}
        if case.get("native_custody"):
            bootstrap["custody_test_url"] = target + case.get("path", "/public")
        configuration = encoded(bootstrap)
        child_streams = [pair[1] for pair in pairs]
        try:
            phase = "launch"
            process = subprocess.Popen([str(host), *(str(stream.fileno()) for stream in child_streams)],
                cwd=allocation, env={"PATH": os.defpath, "HOME": str(allocation), "LANG": "C.UTF-8", "LD_LIBRARY_PATH": str(root)},
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                pass_fds=tuple(stream.fileno() for stream in child_streams), start_new_session=True)
            output = NativeOutput(process, custody_test=bool(case.get("native_custody")))
            for stream in child_streams:
                stream.close()
            phase = "bootstrap"
            pairs[0][0].sendall(key + struct.pack(">I", len(configuration)) + configuration)
            startup_receipt(pairs[0][0], inputs)
            pairs[0][0].close()
            data = Channel(pairs[1][0], key, enrollment, b"data")
            control = Channel(pairs[2][0], key, enrollment, b"control")
            for channel in (data, control):
                phase = "authenticated_ready"
                if channel.request({"operation": "ready"}) != {"result": "ready"}:
                    raise ProbeError("native PKI host did not authenticate readiness")
            session, tab, document = identity("bs"), identity("bt"), identity("bd")
            phase = "open"
            opened = data.request({"operation": "open", "request": {"binding": binding,
                "run_id": "native-pki-fixture-run", "session_id": session, "tab_id": tab,
                "document_id": document, "options": {"mode": "headless", "allowed_origins": origins,
                "initial_url": target + case.get("path", "/public")}}})
            if expected := case.get("title"):
                if opened.get("result") != "opened":
                    code = opened.get("code")
                    category = code if code in ("denied", "unavailable", "failed", "outcome_unknown", "cancelled", "stale", "limit_exceeded") else "invalid_response"
                    raise ProbeError(f"native PKI positive case failed: {case['name']} ({category})")
                summary = opened["tab"]
                for _ in range(20):
                    if summary.get("title") == expected:
                        break
                    phase = "snapshot"
                    snapshot = data.request({"operation": "execute", "command": {"binding": binding,
                        "run_id": "native-pki-fixture-run", "session_id": session,
                        "target": {"tab_id": tab, "document_id": summary["document_id"]},
                        "control_generation": 1, "action": {"kind": "snapshot", "max_nodes": 64},
                        "next_document_id": identity("bd"), "snapshot_id": identity("bn"), "new_tab": None}})
                    if snapshot.get("result") != "observed":
                        raise ProbeError("native PKI could not inspect its fixture document")
                    summary = snapshot["observation"]["tab"]
                    time.sleep(0.05)
                if summary.get("title") != expected:
                    raise ProbeError(f"native PKI fixture title did not match: {case['name']}")
            elif opened.get("result") == "opened":
                if opened.get("tab", {}).get("title") in ("Colossus private CA verified", "Colossus mTLS identity verified"):
                    raise ProbeError("negative native TLS case unexpectedly succeeded")
            elif opened != {"result": "rejected", "code": "outcome_unknown"}:
                raise ProbeError("negative native TLS failure lost its allocation-cleanup obligation")
            if case.get("native_custody"):
                # Loaded/title evidence does not acknowledge the independent
                # native Ready/Challenge/Reply IPC. Keep the owner pumping CEF
                # before Close, within the private challenge's fixed deadline.
                phase = "native_custody"
                live_custody_receipt = output.wait_custody_receipt()
            phase = "close"
            if control.request({"operation": "close", "session_id": session}) != {"result": "acknowledged"}:
                raise ProbeError("native PKI context close was not acknowledged")
            pairs[1][0].close()
            pairs[2][0].close()
            phase = "shutdown"
            output.finish()
            process_group_reaped = True
            if list(allocation.glob("colossus-nss-*")):
                raise ProbeError("owned native NSS HOME survived acknowledged shutdown")
            events = fixture.report()["events"][before:]
            actual_endpoint = case.get("receipt_endpoint", case["endpoint"])
            observed = [event for event in events if event["endpoint"] == actual_endpoint]
            if case.get("title"):
                expected_fp = fixture.metadata["fingerprints_sha256"][case["binding"]] if case.get("binding") else None
                if not any(event["status"] == case["status"] and event["client_fingerprint_sha256"] == expected_fp
                           for event in observed):
                    raise ProbeError("native PKI lacks the actual server-observed expected certificate/status")
            elif not any(event["path"] == "tls_denied" and event["status"] == 0 for event in observed) \
                    or any(event["status"] > 0 or event["client_fingerprint_sha256"] is not None for event in observed):
                raise ProbeError("negative native TLS case lacks server handshake-denial proof")
            if case["name"] == "foreign_redirect" and not any(event["endpoint"] == "mtls" and
                    event["path"] == "/redirect-denied" and event["status"] == 302 and
                    event["client_fingerprint_sha256"] == fixture.metadata["fingerprints_sha256"]["client"]
                    for event in events):
                raise ProbeError("native Chromium lacks the exact bound client identity before the foreign-origin redirect")
            if len(proxy.requests) <= before_tunnels or proxy.overflow or fixture.report()["overflow"]:
                raise ProbeError("native PKI did not authenticate bounded fixture proxy traffic")
            result = {"case": case["name"], "passed": True, "server_receipts": events,
                      "cef_close_acknowledged": True, "native_exit_zero": True, "owned_nss_home_removed": True,
                      "pre_cef_source_inputs_retired": True, "owned_process_group_reaped": True}
            if case.get("native_custody"):
                result["renderer_native_open_denied"] = live_custody_receipt is True \
                    and output.custody_open_denied is True
                # A late shutdown line cannot turn a timed-out live challenge
                # into accepted evidence; subsequent failure remains sticky.
                result["native_receipt"] = "not_reported" if live_custody_receipt is None \
                    else "denied" if result["renderer_native_open_denied"] else "rejected"
                result["native_diagnostics"] = output.custody_diagnostics
                result["passed"] = result["renderer_native_open_denied"]
            return result
        except StartupError as error:
            if output is not None:
                output.settle_failure()
            category = (output.category or output.startup_diagnostic or "unclassified") if output is not None else "unclassified"
            exit_code = process.returncode if process is not None else None
            raise ProbeError(f"native PKI {case['name']} startup failed (phase={error.phase}, native={category}, exit={exit_code})") from None
        except OSError as error:
            if output is not None:
                output.settle_failure()
            category = (output.category or output.startup_diagnostic or "unclassified") if output is not None else "unclassified"
            raise ProbeError(f"native PKI {case['name']} {phase} failed (errno={error.errno}, native={category})") from None
        finally:
            for stream in streams:
                stream.close()
            if process is not None and not process_group_reaped:
                stop_process(process)
            if inputs.quarantined:
                # Unknown replacements must survive the disposable fixture's
                # ordinary cleanup. The owner can inspect this private failed
                # allocation; no source content or location is reported.
                quarantine = parent.parent / f"quarantined-inputs-{uuid.uuid4().hex}"
                private_directory(quarantine)
                os.rename(allocation, quarantine / "allocation")


def run_probe(arguments) -> dict:
    if sys.platform != "linux" or os.geteuid() == 0:
        raise ProbeError("run the native PKI probe as a non-root user in a compatible Linux CEF lane")
    root = component.directory(arguments.component)
    inventory = component.verify_installed(root)
    if inventory["platform"] != "linux64":
        raise ProbeError("native PKI requires the pinned Linux component")
    host = (root / "colossus-native-browser-host").resolve(strict=True)
    if host.parent != root or not os.access(host, os.X_OK):
        raise ProbeError("the verified component must include its native host executable")
    tools = {"certutil": utility(arguments.certutil), "pk12util": utility(arguments.pk12util)}
    host_digest = component.digest_file(host)
    manifest_digest = component.digest_file(root / "browser-component.json")
    evidence = arguments.evidence.absolute()
    private_directory(evidence)
    result = {"schema_version": 1, "native_chromium_tls_conformance": False,
        "owned_nss_cleanup_verified": False, "production_containment": False,
        "signed_installed_artifact_verified": False, "macos_windows_store_custody_verified": False,
        "component": str(root), "component_inventory": inventory,
        "host_sha256": host_digest, "component_manifest_sha256": manifest_digest,
        "nss_utility_sha256": {name: value["sha256"] for name, value in tools.items()},
        "cases": []}
    try:
        with tempfile.TemporaryDirectory(prefix="private-fixture-", dir=evidence) as allocation:
            allocation = Path(allocation)
            fixture = Fixture(allocation / "pki")
            proxy = None
            try:
                fixture.start()
                result["fixture_id"] = fixture.metadata["fixture_id"]
                result["fingerprints_sha256"] = fixture.metadata["fingerprints_sha256"]
                proxy = FixtureProxy(fixture.metadata)
                cases = [
                    {"name": "trusted_ca", "endpoint": "public", "title": "Colossus private CA verified", "status": 200},
                    {"name": "exact_identity", "endpoint": "mtls", "path": "/client-check", "binding": "client",
                     "identities": ["client", "alternate_client"], "title": "Colossus mTLS identity verified", "status": 200},
                    {"name": "alternate_identity", "endpoint": "mtls", "path": "/client-check", "binding": "alternate_client",
                     "identities": ["client", "alternate_client"], "title": "Colossus identity rejected", "status": 403},
                    {"name": "missing_binding", "endpoint": "mtls", "path": "/client-check", "identities": ["client"]},
                    {"name": "untrusted_server", "endpoint": "untrusted"},
                    {"name": "wrong_hostname", "endpoint": "wrong_hostname"},
                    {"name": "expired_server", "endpoint": "expired"},
                    {"name": "foreign_redirect", "endpoint": "mtls", "path": "/redirect-denied", "receipt_endpoint": "redirect_mtls",
                     "binding": "client", "identities": ["client"]},
                    {"name": "pfx_chain_no_autotrust", "endpoint": "public", "trust": False, "identities": ["client_with_ca"]},
                    *[{"name": name, "endpoint": "mtls", "path": "/client-check", "binding": name, "identities": [name]}
                      for name in ("expired_client", "wrong_usage_client", "wrong_issuer_client")],
                ]
                for case in cases:
                    result["case_pending"] = case["name"]
                    result["cases"].append(run_case(case, fixture, proxy, root, host, tools, allocation, inventory))
                    result["case_pending"] = None
                    print(f"PASS native Chromium PKI {case['name']}", file=sys.stderr, flush=True)
            finally:
                result["server_receipts"] = fixture.report()
                result["proxy_tunnels"] = list(proxy.requests) if proxy is not None else []
                if proxy is not None:
                    proxy.close()
                fixture.close()
        if component.verify_installed(root) != inventory \
                or component.digest_file(host) != host_digest \
                or component.digest_file(root / "browser-component.json") != manifest_digest:
            raise ProbeError("native PKI component changed during its acceptance matrix")
        result["native_chromium_tls_conformance"] = True
        result["owned_nss_cleanup_verified"] = True
        return result
    finally:
        private_write(evidence / "native-pki-acceptance.json", (json.dumps(result, indent=2) + "\n").encode())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--component", type=Path, required=True)
    parser.add_argument("--certutil", type=Path, required=True)
    parser.add_argument("--pk12util", type=Path, required=True)
    parser.add_argument("--evidence", type=Path,
        default=Path.cwd() / ".local" / f"native-pki-acceptance-{uuid.uuid4()}")
    arguments = parser.parse_args()
    try:
        print(json.dumps(run_probe(arguments), sort_keys=True))
        return 0
    except ProbeError as error:
        print(str(error), file=sys.stderr)
        return 1
    except component.ComponentError as error:
        print(f"native PKI component prerequisite failed: {error}", file=sys.stderr)
        return 1
    except FileNotFoundError:
        print("native PKI component, utility, or evidence parent was missing", file=sys.stderr)
        return 1
    except PermissionError:
        print("native PKI prerequisite ownership or filesystem permission failed", file=sys.stderr)
        return 1
    except ValueError as error:
        print(f"native PKI prerequisite or fixture rejected ({type(error).__name__})", file=sys.stderr)
        return 1
    except OSError as error:
        print(f"native PKI bounded filesystem/socket operation failed (errno={error.errno})", file=sys.stderr)
        return 1
    except subprocess.TimeoutExpired:
        print("native PKI owned process did not settle before its deadline", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
