#!/usr/bin/env python3
"""Stage and run the exact Debug-only Core Mac Chromium HTTP fixture.

This source diagnostic does not enable production automation, certificate custody,
whole-helper cleanup, or parent-crash acceptance. Every allocation is preserved.
The caller must select the repository Rust toolchain before invoking this script.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import platform
import re
import socket
import stat
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import urlsplit

import component
import stage_macos


ROOT = Path(__file__).resolve().parents[3]
IDENTIFIER = "com.obscuritylabs.colossus.native-browser-host.preview"
CORE_TEST = (
    "browser_tools::tests::native_macos::"
    "real_mac_chromium_executes_core_tools_and_paints_authenticated_frames"
)
BUILD_TIMEOUT = 1800
MAX_TEST_LOG = 32 * 1024 * 1024


class DiagnosticError(ValueError):
    """A fixed diagnostic prerequisite or evidence failure."""


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--cef-root", type=Path, required=True)
    result.add_argument("--native-build", type=Path, required=True)
    result.add_argument("--host-binary", type=Path, required=True)
    result.add_argument("--output-root", type=Path, required=True,
                        help="owner-private evidence parent; allocations are never removed")
    result.add_argument("--stage-only", action="store_true",
                        help="prepare a fresh signed allocation and Desktop compiler seal without launching a browser")
    result.add_argument("--fixture-origin",
                        help="stage-only Desktop fixture: one canonical http://127.0.0.1:<nondefault-port>; otherwise choose a free port")
    return result


def macos_platform() -> str:
    if sys.platform != "darwin":
        raise DiagnosticError("this diagnostic requires native macOS")
    machine = platform.machine().lower()
    if machine in ("arm64", "aarch64"):
        return "macosarm64"
    if machine == "x86_64":
        return "macosx64"
    raise DiagnosticError("unsupported macOS architecture")


def policy_path(path: Path) -> str:
    value = str(path)
    if not path.is_absolute() or not value.isascii() or any(ord(char) < 32 for char in value):
        raise DiagnosticError("network policy requires absolute ASCII paths without control characters")
    return value


def existing_path(path: Path, *, directory: bool) -> Path:
    path = path.expanduser().resolve(strict=True)
    metadata = path.stat()
    if (directory and not stat.S_ISDIR(metadata.st_mode)) or (
            not directory and (not stat.S_ISREG(metadata.st_mode) or not metadata.st_mode & 0o111)):
        raise DiagnosticError("diagnostic input has the wrong file kind or mode")
    return path


def private_parent(path: Path) -> Path:
    path = path.expanduser().absolute()
    if path.is_symlink():
        raise DiagnosticError("evidence parent must not be a symlink")
    path = path.parent.resolve(strict=True) / path.name
    try:
        path.mkdir(mode=0o700)
    except FileExistsError:
        pass
    metadata = path.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or
            stat.S_IMODE(metadata.st_mode) != 0o700):
        raise DiagnosticError("evidence parent must be an owned 0700 directory")
    policy_path(path)
    return path


def private_child(parent: Path, prefix: str) -> Path:
    path = Path(tempfile.mkdtemp(prefix=prefix, dir=parent)).resolve(strict=True)
    path.chmod(0o700)
    return path


def private_file(path: Path):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                         os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    os.fchmod(descriptor, 0o600)
    return os.fdopen(descriptor, "wb")


def write_private(path: Path, data: bytes) -> None:
    with private_file(path) as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())


def write_json(path: Path, value: dict) -> None:
    write_private(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8"))


def run_logged(arguments: list[str], environment: dict[str, str], log: Path,
               *, timeout: int, capture: bool = False) -> bytes:
    with private_file(log) as output:
        result = subprocess.run(arguments, cwd=ROOT, env=environment, check=True,
                                timeout=timeout, stdout=subprocess.PIPE if capture else output,
                                stderr=output)
        if capture:
            output.write(result.stdout)
        output.flush()
        os.fsync(output.fileno())
    return result.stdout if capture else b""


def require_toolchain(environment: dict[str, str], evidence: Path) -> None:
    expected = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    for executable in ("rustc", "cargo"):
        version = run_logged([executable, "--version"], environment,
                             evidence / f"{executable}-version.log", timeout=15, capture=True)
        if len(version) > 8192 or not version.startswith(f"{executable} {expected} ".encode()):
            raise DiagnosticError("select the repository Rust and Cargo toolchain in PATH")


def current_audit_session(environment: dict[str, str], evidence: Path) -> int:
    # This example only reads this process's live identity. No audit-session port,
    # join, wrong-port probe, session transfer, or ignored test is invoked.
    output = run_logged([
        "cargo", "run", "--quiet", "--locked", "-p", "colossus-darwin-process",
        "--example", "browser_current_audit_session",
    ], environment, evidence / "current-identity.log", timeout=BUILD_TIMEOUT, capture=True)
    if len(output) > 32 or re.fullmatch(rb"[0-9]+\n?", output) is None:
        raise DiagnosticError("current audit-session identity was not a bounded integer")
    value = int(output)
    if not 0 < value < 2**32 - 1:
        raise DiagnosticError("current audit-session identity is unavailable")
    return value


@contextmanager
def reserve_loopbacks(port: int = 0, *, ipv6: bool = True):
    ipv4_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    ipv6_socket = None
    try:
        ipv4_socket.bind(("127.0.0.1", port))
        selected = ipv4_socket.getsockname()[1]
        if ipv6:
            ipv6_socket = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
            ipv6_socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
            ipv6_socket.bind(("::1", selected))
        yield selected
    finally:
        if ipv6_socket is not None:
            ipv6_socket.close()
        ipv4_socket.close()


def fixture_port(value: str | None) -> int:
    if value is None:
        return 0
    parsed = urlsplit(value)
    try:
        port = parsed.port
    except ValueError as error:
        raise DiagnosticError("invalid fixed fixture port") from error
    if (port is None or port in (0, 80) or parsed.scheme != "http" or
            parsed.hostname != "127.0.0.1" or parsed.username is not None or
            parsed.password is not None or parsed.path or parsed.query or parsed.fragment or
            value != f"http://127.0.0.1:{port}"):
        raise DiagnosticError("fixture requires one canonical numeric loopback HTTP origin")
    return port


def policy_bytes(session: int, port: int, allocation: Path, app: Path,
                 broker: Path, personal: Path) -> bytes:
    if not 0 < session < 2**32 - 1 or not 0 < port < 65536:
        raise DiagnosticError("invalid fixed network-envelope identity or port")
    return ("\n".join((stage_macos.NETWORK_ENVELOPE_MARKER, str(session), str(port),
                       policy_path(allocation), policy_path(app), IDENTIFIER,
                       policy_path(allocation / "profile"), policy_path(broker),
                       policy_path(personal), ""))).encode("ascii")


def stage_command(cef_root: Path, native_build: Path, host: Path, app: Path,
                  target: str, policy: Path) -> list[str]:
    return [
        sys.executable, "-B", str(ROOT / "native/browser/scripts/stage_host_macos.py"),
        "--cef-root", str(cef_root), "--native-build", str(native_build),
        "--executable", str(host), "--app", str(app), "--platform", target,
        "--macos-synthetic-profile-diagnostic", "--macos-network-envelope-policy", str(policy),
    ]


def core_command() -> list[str]:
    return [
        "cargo", "test", "--locked", "-p", "colossus-runtime", "--features",
        "macos-browser-host-acceptance", "--lib", CORE_TEST,
        "--", "--ignored", "--exact", "--nocapture", "--color", "never",
    ]


def verify_core_result(evidence: Path) -> None:
    # Rust's test harness exits successfully when an exact filter matches zero
    # tests. A source diagnostic must prove that its one named fixture ran.
    with (evidence / "core-test.log").open("rb") as source:
        log = source.read(MAX_TEST_LOG + 1)
    if (len(log) > MAX_TEST_LOG or
            re.search(rb"^test " + re.escape(CORE_TEST.encode()) + rb" \.\.\. ok\r?$",
                      log, re.MULTILINE) is None or
            re.search(rb"^test result: ok\. 1 passed; 0 failed; 0 ignored;", log,
                      re.MULTILINE) is None):
        raise DiagnosticError("the exact Core fixture did not produce a successful test receipt")
    with (evidence / "mac-core.png").open("rb") as source:
        if source.read(8) != b"\x89PNG\r\n\x1a\n":
            raise DiagnosticError("the successful fixture did not preserve its PNG evidence")


def cdhash(executable: Path, environment: dict[str, str], evidence: Path, name: str) -> str:
    with private_file(evidence / name) as output:
        result = subprocess.run(["/usr/bin/codesign", "-d", "--verbose=4", str(executable)],
                                cwd=ROOT, env=environment, check=True, timeout=15,
                                capture_output=True)
        output.write(result.stdout)
        output.write(result.stderr)
        output.flush()
        os.fsync(output.fileno())
    match = re.search(rb"^CDHash=([0-9a-f]{40})$", result.stderr, re.MULTILINE)
    if match is None:
        raise DiagnosticError("signed stage has no exact cdhash")
    return match.group(1).decode("ascii")


def digest(path: Path) -> str:
    return component.digest_file(path)


def personal_home() -> Path:
    import pwd
    return Path(pwd.getpwuid(os.geteuid()).pw_dir).resolve(strict=True)


def prepare(options: argparse.Namespace) -> tuple[Path, dict[str, str]]:
    target = macos_platform()
    if options.fixture_origin is not None and not options.stage_only:
        raise DiagnosticError("--fixture-origin is reserved for --stage-only Desktop preparation")
    port = fixture_port(options.fixture_origin)
    cef_root = existing_path(options.cef_root, directory=True)
    native_build = existing_path(options.native_build, directory=True)
    host = existing_path(options.host_binary, directory=False)
    if host.name != "colossus-native-browser-host":
        raise DiagnosticError("the diagnostic requires the fixed native host executable")
    if (component.load_json(native_build / stage_macos.SYNTHETIC_PROFILE_BUILD) !=
            stage_macos.SYNTHETIC_PROFILE_METADATA or
            component.load_json(native_build / stage_macos.NETWORK_ENVELOPE_BUILD) !=
            stage_macos.NETWORK_ENVELOPE_METADATA or
            (native_build / stage_macos.PROFILE_CRYPTO_BUILD).exists()):
        raise DiagnosticError("use a separate Debug synthetic-profile and network-envelope build")
    parent = private_parent(options.output_root)
    evidence = private_child(parent, "evidence-")
    print(f"Diagnostic evidence retained at {evidence}", flush=True)
    allocation = private_child(parent, "allocation-")
    broker = private_child(parent, "broker-")
    cargo_tmp = private_child(evidence, "cargo-tmp-")
    environment = dict(os.environ)
    environment["TMPDIR"] = str(cargo_tmp)
    require_toolchain(environment, evidence)
    session = current_audit_session(environment, evidence)
    personal = personal_home()
    app = allocation / "Colossus Core Browser.app"
    policy = allocation / "binding.source"
    with reserve_loopbacks() as proxy_port, reserve_loopbacks(port, ipv6=False) as page_port:
        write_private(policy, policy_bytes(session, proxy_port, allocation, app, broker, personal))
        run_logged(stage_command(cef_root, native_build, host, app, target, policy),
                   environment, evidence / "staging.log", timeout=BUILD_TIMEOUT)
        front = app / "Contents/MacOS/colossus-native-browser-host"
        manifest_hash = digest(app.with_name(app.name + ".browser-component.json"))
        front_hash = cdhash(front, environment, evidence, "front-signature.log")
        body_hash = cdhash(front.with_name(front.name + " Body"), environment,
                           evidence, "body-signature.log")
        policy_hash = digest(app / "Contents/Resources" / stage_macos.NETWORK_ENVELOPE_RESOURCE)
        stage_environment = {
            "COLOSSUS_BROWSER_MACOS_ACCEPTANCE_APP": str(app),
            "COLOSSUS_BROWSER_MACOS_MANIFEST_SHA256": manifest_hash,
            "COLOSSUS_BROWSER_MACOS_FRONT_CDHASH": front_hash,
            "COLOSSUS_BROWSER_MACOS_BODY_CDHASH": body_hash,
            "COLOSSUS_BROWSER_MACOS_POLICY_SHA256": policy_hash,
            "COLOSSUS_BROWSER_MACOS_ACCEPTANCE_SCREENSHOT": str(evidence / "mac-core.png"),
        }
        write_json(evidence / "stage-environment.json", stage_environment)
        write_json(evidence / "desktop-compiler-seal.json", {
            "allocation_root": str(allocation), "app_bundle": str(app),
            "manifest_sha256": manifest_hash, "front_cdhash": front_hash,
            "body_cdhash": body_hash, "policy_sha256": policy_hash,
            "fixture_origin": f"http://127.0.0.1:{page_port}",
        })
        write_json(evidence / "diagnostic-stage.json", {
            "schema_version": 1, "development_only": True, "production_accepted": False,
            "whole_helper_cleanup_accepted": False, "certificate_custody_accepted": False,
            "platform": target, "allocation_root": str(allocation), "broker_root": str(broker),
            "app_bundle": str(app), "core_test": CORE_TEST,
            "stage_only": options.stage_only,
        })
    # The sealed listeners must be released before Core binds its exact proxy;
    # occupying either port later causes admission to fail instead of substituting.
    environment.update(stage_environment)
    return evidence, environment


def main(arguments: list[str] | None = None) -> int:
    options = parser().parse_args(arguments)
    evidence = None
    try:
        evidence, environment = prepare(options)
        if options.stage_only:
            print(f"Fresh Desktop compiler seal: {evidence / 'desktop-compiler-seal.json'}")
            print("Stage only; no browser launched, no native acceptance established.")
            return 0
        run_logged(core_command(), environment, evidence / "core-test.log", timeout=BUILD_TIMEOUT)
        verify_core_result(evidence)
        print(f"Exact Core diagnostic exited successfully; inspect {evidence / 'core-test.log'}")
        print("Production automation and whole-helper/crash/PKI acceptance remain disabled.")
        return 0
    except (DiagnosticError, component.ComponentError, OSError, ValueError,
            subprocess.SubprocessError):
        print("Core Mac diagnostic failed; generated allocations and logs remain retained.", file=sys.stderr)
        if evidence is not None:
            print(f"Inspect {evidence}; no helper-tree cleanup is inferred.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
