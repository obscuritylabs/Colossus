#!/usr/bin/env python3
"""Build and sign a fresh owned Core/Chromium HTTPS fixture without launching it.

The Core test embeds independent public trust before native broker compilation.
Every allocation and log is retained. Only the preparer's exact signing seed is
removed after successful signing; synthetic fixture keys stay outside the browser
allocation. Native acceptance and production readiness are never inferred.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
from urllib.parse import urlsplit

import component
import owned_process_component
import run_core_macos_diagnostic as common
import stage_macos

ROOT = common.ROOT
PURPOSE = "colossus-macos-owned-process-development"
CORE_TEST = ("browser_tools::tests::native_macos::owned_https::"
             "real_owned_mac_chromium_controls_https_through_core_and_reaps_known_helpers")
TRUST_ENV = "COLOSSUS_MACOS_OWNED_ACCEPTANCE_TRUST"
INPUTS_ENV = "COLOSSUS_MACOS_OWNED_ACCEPTANCE_INPUTS"
# The signed CBOOT role differentiates network/utility in the same fixed generic
# helper image. Keep this tuple aligned with native Installation::bind, not the
# complete five-bundle inventory used by CEF helper discovery.
HELPER_SUFFIXES = (" (Renderer)", " (GPU)", "", "")
MAX_BUILD_JSON = 64 * 1024 * 1024


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description=__doc__)
    value.add_argument("--cef-root", required=True, type=Path)
    value.add_argument("--native-build", required=True, type=Path)
    value.add_argument("--host-binary", required=True, type=Path)
    value.add_argument("--output-root", required=True, type=Path)
    value.add_argument("--cargo-target-dir", type=Path,
                       help="explicit owner-private compiler cache; default is a fresh evidence child")
    value.add_argument("--fixture-origin",
                       help="one canonical https://127.0.0.1:nondefault-port; otherwise reserve a fresh port")
    return value


def fixture_port(value: str | None) -> int:
    if value is None:
        return 0
    parsed = urlsplit(value)
    try:
        port = parsed.port
    except ValueError as error:
        raise common.DiagnosticError("invalid HTTPS fixture port") from error
    if (port is None or port in (0, 443) or parsed.scheme != "https" or
            parsed.hostname != "127.0.0.1" or parsed.username is not None or
            parsed.password is not None or parsed.path or parsed.query or parsed.fragment or
            value != f"https://127.0.0.1:{port}"):
        raise common.DiagnosticError("fixture requires one canonical numeric loopback HTTPS origin")
    return port


def fixture_certificates(inputs: Path) -> bytes:
    # These are explicit synthetic fixture keys, not a reviewed client identity
    # or a system/private Keychain operation. No trust store is changed.
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import ec
    from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

    now = datetime.now(timezone.utc)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    issuer = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Colossus owned HTTPS fixture CA")])
    ca = (x509.CertificateBuilder().subject_name(issuer).issuer_name(issuer)
          .public_key(ca_key.public_key()).serial_number(x509.random_serial_number())
          .not_valid_before(now - timedelta(minutes=5)).not_valid_after(now + timedelta(hours=4))
          .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
          .add_extension(x509.KeyUsage(False, False, False, False, False, True, True,
                                      False, False), critical=True)
          .sign(ca_key, hashes.SHA256()))
    key = ec.generate_private_key(ec.SECP256R1())
    leaf = (x509.CertificateBuilder().subject_name(x509.Name([
        x509.NameAttribute(NameOID.COMMON_NAME, "127.0.0.1")])).issuer_name(issuer)
        .public_key(key.public_key()).serial_number(x509.random_serial_number())
        .not_valid_before(now - timedelta(minutes=5)).not_valid_after(now + timedelta(hours=4))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.SubjectAlternativeName([
            x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]), critical=False)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .add_extension(x509.KeyUsage(True, False, False, False, False, False, False,
                                    False, False), critical=True)
        .sign(ca_key, hashes.SHA256()))
    public_root = ca.public_bytes(serialization.Encoding.DER)
    common.write_private(inputs / "fixture-ca.der", public_root)
    common.write_private(inputs / "fixture-leaf.der", leaf.public_bytes(serialization.Encoding.DER))
    common.write_private(inputs / "fixture-key.pk8", key.private_bytes(
        serialization.Encoding.DER, serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption()))
    return public_root


def core_build_command() -> list[str]:
    return ["cargo", "test", "--locked", "-p", "colossus-runtime", "--features",
            "macos-browser-host-acceptance", "--lib", "--no-run", "--message-format=json",
            CORE_TEST]


def core_executable(output: bytes) -> Path:
    if len(output) > MAX_BUILD_JSON:
        raise common.DiagnosticError("Core compiler output exceeds its bound")
    executables = set()
    for line in output.splitlines():
        try:
            value = json.loads(line)
        except (ValueError, UnicodeError):
            continue
        if (value.get("reason") == "compiler-artifact" and
                value.get("target", {}).get("name") == "colossus_runtime" and
                value.get("target", {}).get("kind") == ["lib"] and
                value.get("profile", {}).get("test") is True and value.get("executable")):
            executables.add(value["executable"])
    if len(executables) != 1:
        raise common.DiagnosticError("compiler did not identify one exact Runtime lib-test executable")
    path = Path(executables.pop())
    if not path.is_absolute():
        raise common.DiagnosticError("compiler executable must be absolute")
    return path


def compile_core(environment: dict[str, str], log: Path) -> bytes:
    # Cargo emits structured Rust diagnostics on stdout. Preserve them before
    # raising a compilation failure; check=True would discard this evidence.
    with common.private_file(log) as output:
        result = subprocess.run(core_build_command(), cwd=ROOT, env=environment,
                                timeout=common.BUILD_TIMEOUT, stdout=subprocess.PIPE,
                                stderr=output, check=False)
        if len(result.stdout) > MAX_BUILD_JSON:
            raise common.DiagnosticError("Core compiler output exceeds its bound")
        output.write(result.stdout)
        output.flush()
        os.fsync(output.fileno())
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, core_build_command())
    return result.stdout


def native_build_command(kind: str) -> list[str]:
    if kind == "process":
        package, feature = "process-broker", "macos-process-entry-development,macos-process-output-diagnostic"
    elif kind == "tls":
        package, feature = "tls-broker", "macos-https-broker-entry-development"
    else:
        raise common.DiagnosticError("unsupported native build kind")
    return ["cargo", "build", "--locked", "--offline", "--manifest-path",
            str(ROOT / "native/browser" / package / "Cargo.toml"), "--features", feature]


def stage_command(cef: Path, native: Path, host: Path, app: Path, target: str,
                  policy: Path, process: Path, tls: Path) -> list[str]:
    return [sys.executable, "-B", str(ROOT / "native/browser/scripts/stage_host_macos.py"),
            "--cef-root", str(cef), "--native-build", str(native), "--executable", str(host),
            "--app", str(app), "--platform", target, "--owned-process-development",
            "--process-broker", str(process), "--tls-broker", str(tls),
            "--macos-synthetic-profile-diagnostic", "--macos-network-envelope-policy", str(policy)]


def descriptor(allocation: Path, app: Path, origin: str, root: bytes,
               facts: dict[str, str], helpers: list[str]) -> dict:
    port = fixture_port(origin)
    if not root or len(root) > 8192 or len(helpers) != 4:
        raise common.DiagnosticError("invalid fixed fixture roots or helper identities")
    return {"schema_version": 1, "purpose": PURPOSE, "production_accepted": False,
            "allocation_root": str(allocation), "app_bundle": str(app),
            "manifest_sha256": facts["manifest_sha256"], "front_cdhash": facts["front_cdhash"],
            "body_cdhash": facts["body_cdhash"], "policy_sha256": facts["policy_sha256"],
            "fixture_origin": origin, "broker_cdhash": facts["broker_cdhash"],
            "helper_cdhashes": helpers, "custody_denials_sha256": None,
            "tls": {"broker_cdhash": facts["tls_cdhash"],
                    "destinations": [f"127.0.0.1:{port}"], "public_roots": [list(root)]}}


def seed_identity(path: Path) -> tuple[int, int]:
    value = path.lstat()
    if (not stat.S_ISREG(value.st_mode) or value.st_uid != os.geteuid() or
            stat.S_IMODE(value.st_mode) != 0o600 or value.st_nlink != 1 or value.st_size != 32):
        raise common.DiagnosticError("unsafe development signing seed")
    return value.st_dev, value.st_ino


def remove_owned_seed(path: Path, identity: tuple[int, int]) -> None:
    if seed_identity(path) != identity:
        raise common.DiagnosticError("signing seed changed; retain it for inspection")
    path.unlink()


def sign_core(core: Path, environment: dict[str, str], evidence: Path) -> str:
    common.run_logged(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none",
                       "--options", "runtime", str(core)], environment,
                      evidence / "core-sign.log", timeout=30)
    common.run_logged(["/usr/bin/codesign", "--verify", "--strict", str(core)], environment,
                      evidence / "core-verify.log", timeout=30)
    return common.cdhash(core, environment, evidence, "core-signature.log")


def freeze_core(compiled: Path, evidence: Path) -> Path:
    # Cargo can overwrite its deterministic test-executable path in a later
    # build. Freeze a separate single-link copy before signing/native sealing.
    before = common.digest(compiled)
    frozen = common.private_child(evidence, "frozen-core-") / compiled.name
    with compiled.open("rb") as source, common.private_file(frozen) as output:
        shutil.copyfileobj(source, output, length=1024 * 1024)
        output.flush()
        os.fsync(output.fileno())
    frozen.chmod(0o700)
    if common.digest(frozen) != before or common.digest(compiled) != before:
        raise common.DiagnosticError("Core compiler artifact changed during freezing")
    return frozen


def prepare(options: argparse.Namespace) -> Path:
    target = common.macos_platform()
    cef = common.existing_path(options.cef_root, directory=True)
    native = common.existing_path(options.native_build, directory=True)
    host = common.existing_path(options.host_binary, directory=False)
    if host.name != "libcolossus_native_browser_host.dylib":
        raise common.DiagnosticError("owned fixture requires the fixed Main body dylib")
    owned_process_component.verify_build(cef, native, target)
    if (component.load_json(native / stage_macos.SYNTHETIC_PROFILE_BUILD) !=
            stage_macos.OWNED_PROFILE_METADATA or
            component.load_json(native / stage_macos.NETWORK_ENVELOPE_BUILD) !=
            stage_macos.NETWORK_ENVELOPE_METADATA or (native / stage_macos.PROFILE_CRYPTO_BUILD).exists()):
        raise common.DiagnosticError("requires separate owned Debug synthetic/network build")
    parent = common.private_parent(options.output_root)
    evidence = common.private_child(parent, "evidence-")
    print(f"Owned preparation evidence retained at {evidence}", flush=True)
    allocation = common.private_child(parent, "allocation-")
    broker = common.private_child(parent, "broker-")
    inputs = common.private_child(broker, "acceptance-inputs-")
    signing = common.private_child(broker, "signing-")
    seed = signing / "seed"
    environment = dict(os.environ)
    for name in ("COLOSSUS_SIDECAR_BROWSER_MANIFEST", "COLOSSUS_SIDECAR_MAC_BROWSER_ACCEPTANCE_SEAL",
                 "COLOSSUS_MAC_PROCESS_PARENT_CDHASH", "COLOSSUS_MAC_PROCESS_DIAGNOSTIC_ROOT", "COLOSSUS_TLS_BROKER_PARENT_CDHASH",
                 TRUST_ENV, INPUTS_ENV):
        environment.pop(name, None)
    environment["TMPDIR"] = str(common.private_child(evidence, "cargo-tmp-"))
    environment["CARGO_BUILD_JOBS"] = "2"
    environment["CARGO_TARGET_DIR"] = str(common.private_parent(options.cargo_target_dir)
        if options.cargo_target_dir else common.private_child(evidence, "core-build-"))
    common.require_toolchain(environment, evidence)
    session = common.current_audit_session(environment, evidence)
    signer_command = ["cargo", "run", "--quiet", "--locked", "-p", "colossus-runtime",
                      "--features", "macos-browser-host-acceptance", "--example",
                      "macos_owned_process_package", "--"]
    common.run_logged([*signer_command, "trust", str(allocation), str(seed)], environment,
                      evidence / "trust-signing.log", timeout=common.BUILD_TIMEOUT)
    identity = seed_identity(seed)
    trust = component.load_json(allocation / "compiler-trust.json")
    if set(trust) != {"schema_version", "purpose", "publisher", "package_root", "key_id", "public_key"}:
        raise common.DiagnosticError("compiler trust has an unexpected schema")
    environment[TRUST_ENV] = json.dumps(trust, separators=(",", ":"), sort_keys=True)
    environment[INPUTS_ENV] = str(inputs)
    public_root = fixture_certificates(inputs)
    core_json = compile_core(environment, evidence / "core-build.log")
    core = freeze_core(common.existing_path(core_executable(core_json), directory=False), evidence)
    core_hash = sign_core(core, environment, evidence)
    native_environment = dict(environment)
    native_target = common.private_child(evidence, "native-entry-build-")
    native_environment["CARGO_TARGET_DIR"] = str(native_target)
    native_environment["COLOSSUS_MAC_PROCESS_PARENT_CDHASH"] = core_hash
    diagnostic = broker / "process-diagnostics"
    diagnostic.mkdir(mode=0o700)
    native_environment["COLOSSUS_MAC_PROCESS_DIAGNOSTIC_ROOT"] = str(diagnostic)
    native_environment["COLOSSUS_TLS_BROKER_PARENT_CDHASH"] = core_hash
    for kind in ("process", "tls"):
        common.run_logged(native_build_command(kind), native_environment,
                          evidence / f"{kind}-build.log", timeout=common.BUILD_TIMEOUT)
    process = native_target / "debug/colossus-mac-browser-process-broker"
    tls = native_target / "debug/colossus-native-browser-tls-broker"
    app = allocation / "Colossus Owned Core Browser.app"
    policy = allocation / "binding.source"
    with common.reserve_loopbacks() as proxy_port, common.reserve_loopbacks(
            fixture_port(options.fixture_origin), ipv6=False) as page_port:
        origin = f"https://127.0.0.1:{page_port}"
        common.write_private(policy, common.policy_bytes(
            session, proxy_port, allocation, app, broker, common.personal_home()))
        common.run_logged(stage_command(cef, native, host, app, target, policy, process, tls),
                          environment, evidence / "staging.log", timeout=common.BUILD_TIMEOUT)
        macos = app / "Contents/MacOS"
        front = macos / "colossus-native-browser-host"
        facts = {"manifest_sha256": common.digest(app.with_name(app.name + ".browser-component.json")),
                 "policy_sha256": common.digest(app / "Contents/Resources" / stage_macos.NETWORK_ENVELOPE_RESOURCE)}
        for key, path in (("front", front), ("body", front.with_name(front.name + " Body.dylib")),
                          ("broker", macos / process.name), ("tls", macos / tls.name)):
            facts[key + "_cdhash"] = common.cdhash(path, environment, evidence, key + "-signature.log")
        helpers = []
        for index, suffix in enumerate(HELPER_SUFFIXES):
            name = stage_macos.HELPER + suffix
            helper = app / "Contents/Frameworks" / (name + ".app") / "Contents/MacOS" / name
            helpers.append(common.cdhash(helper, environment, evidence, f"helper-{index}-signature.log"))
        common.write_json(allocation / "owned-process-host.json",
                          descriptor(allocation, app, origin, public_root, facts, helpers))
        common.run_logged([*signer_command, "sign", str(allocation), str(seed)], environment,
                          evidence / "descriptor-signing.log", timeout=common.BUILD_TIMEOUT)
        remove_owned_seed(seed, identity)
        # Signing the native entries and external descriptor must not mutate the
        # independently sealed Core executable. Never rebuild it after sealing.
        if common.cdhash(core, environment, evidence, "core-final-signature.log") != core_hash:
            raise common.DiagnosticError("sealed Core identity changed during package preparation")
        launch = [str(core), CORE_TEST, "--ignored", "--exact", "--nocapture", "--color", "never"]
        common.write_json(evidence / "owned-core-stage.json", {
            "schema_version": 1, "development_only": True, "production_accepted": False,
            "native_acceptance": False, "full_helper_cleanup_proven": False,
            "certificate_custody_accepted": False, "signing_seed_removed": True,
            "allocation_root": str(allocation), "broker_root": str(broker),
            "private_process_diagnostics": native_environment["COLOSSUS_MAC_PROCESS_DIAGNOSTIC_ROOT"],
            "fixture_inputs": str(inputs), "fixture_origin": origin, "app_bundle": str(app),
            "core_executable": str(core), "core_cdhash": core_hash,
            "compiler_trust_sha256": common.digest(allocation / "compiler-trust.json"),
            "descriptor_sha256": common.digest(allocation / "owned-process-host.json"),
            "command": launch, "cwd": str(ROOT), "fixture_port_released": True,
        })
    print(f"Stage only; Root-owned exact native launch facts: {evidence / 'owned-core-stage.json'}")
    return evidence


def main(arguments: list[str] | None = None) -> int:
    try:
        prepare(parser().parse_args(arguments))
        return 0
    except (common.DiagnosticError, component.ComponentError, OSError, ValueError,
            ImportError, subprocess.SubprocessError):
        print("Owned Core preparation failed; allocations, signing material and logs retained.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
