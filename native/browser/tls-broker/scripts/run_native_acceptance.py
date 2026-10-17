#!/usr/bin/env python3
"""Prepare or explicitly run the sealed Mac TLS-broker toy-parent fixture.

Default preparation builds/signs images only. Native launches require --run and
remain separate from production browser availability, identity custody and CEF.
All allocations and failure evidence are retained.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[4]
MANIFEST = ROOT / "native/browser/tls-broker/Cargo.toml"
CASES = ("positive", "eof", "wrong-parent", "parent-crash")
MAX_TOOL_OUTPUT = 16 * 1024 * 1024


class AcceptanceError(ValueError):
    """Categorical prerequisite or evidence rejection."""


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    action = result.add_mutually_exclusive_group()
    action.add_argument("--prepare-only", action="store_true",
                        help="default: build/sign a fresh allocation without native launches")
    action.add_argument("--run", action="store_true",
                        help="explicitly launch the recorded native toy-parent cases")
    result.add_argument("--output-root", type=Path)
    result.add_argument("--prepared", type=Path)
    result.add_argument("--case", choices=(*CASES, "all"), default="all")
    return result


def require_platform() -> None:
    if sys.platform != "darwin" or platform.machine().lower() not in ("arm64", "aarch64", "x86_64"):
        raise AcceptanceError("native TLS acceptance requires supported macOS")
    if os.geteuid() == 0 or os.geteuid() != os.getuid():
        raise AcceptanceError("native TLS acceptance requires an unprivileged owner")


def private_directory(path: Path) -> Path:
    path = path.expanduser().absolute()
    if path.is_symlink():
        raise AcceptanceError("evidence directory must not be a symlink")
    path = path.parent.resolve(strict=True) / path.name
    try:
        path.mkdir(mode=0o700)
    except FileExistsError:
        pass
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise AcceptanceError("evidence directory must be owned and private")
    return path


def write_private(path: Path, value: bytes) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(value)
        output.flush()
        os.fsync(output.fileno())


def write_json(path: Path, value: dict) -> None:
    write_private(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode())


def run_tool(argv: list[str], environment: dict[str, str], log: Path, timeout: int = 1800) -> bytes:
    result = subprocess.run(argv, cwd=ROOT, env=environment, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=timeout, check=False)
    if len(result.stdout) > MAX_TOOL_OUTPUT:
        write_private(log, result.stdout[:MAX_TOOL_OUTPUT])
        raise AcceptanceError("fixture tool output exceeded its evidence bound")
    write_private(log, result.stdout)
    if result.returncode:
        raise AcceptanceError("fixture build, signing or native assertion failed; inspect retained log")
    return result.stdout


def supported_toolchain(environment: dict[str, str], run_root: Path) -> None:
    for name in ("rustc", "cargo"):
        output = run_tool([name, "--version"], environment, run_root / f"{name}-version.log", 30)
        if not output.decode("ascii", errors="strict").startswith(name + " 1.96."):
            raise AcceptanceError("select the pinned Rust 1.96 toolchain before preparing")


def source_digests() -> dict[str, str]:
    paths = {ROOT / "Cargo.toml", ROOT / "rust-toolchain.toml"}
    for directory in (ROOT / "native/browser/tls-broker", ROOT / "native/browser/pki", ROOT / "crates/colossus-darwin-process"):
        paths.update(directory.glob("*.toml"))
        paths.update(directory.glob("*.lock"))
        paths.update(directory.glob("build.rs"))
        paths.update(directory.glob("src/**/*.rs"))
        paths.update(directory.glob("examples/**/*.rs"))
        paths.update(directory.glob("scripts/*.py"))
    return {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(paths)}


def code_hash(image: Path, environment: dict[str, str], log: Path) -> str:
    output = run_tool(["/usr/bin/codesign", "-d", "--verbose=4", str(image)], environment, log, 30)
    hashes = re.findall(rb"(?m)^CDHash=([0-9a-fA-F]{40})$", output)
    if len(hashes) != 1 or hashes[0] == b"0" * 40:
        raise AcceptanceError("signed image did not expose one exact nonzero code hash")
    return hashes[0].decode("ascii").lower()


def signed_image(source: Path, destination: Path, identifier: str,
                 environment: dict[str, str], run_root: Path) -> dict:
    shutil.copyfile(source, destination, follow_symlinks=False)
    destination.chmod(0o700)
    run_tool(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", "--identifier", identifier, str(destination)],
             environment, run_root / (destination.name + "-sign.log"), 30)
    run_tool(["/usr/bin/codesign", "--verify", "--strict", str(destination)], environment,
             run_root / (destination.name + "-verify.log"), 30)
    destination.chmod(0o500)
    return {"path": str(destination), "sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
            "cdhash": code_hash(destination, environment, run_root / (destination.name + "-identity.log"))}


def wrong_hash(parent_cdhash: str) -> str:
    value = hashlib.sha256(b"colossus-native-tls-wrong-parent-fixture-v1\0" + bytes.fromhex(parent_cdhash)).digest()[:20].hex()
    if value == parent_cdhash or value == "0" * 40:
        raise AcceptanceError("wrong-parent fixture identity did not differ")
    return value


def case_commands(run_root: Path, parent: dict, broker: dict, wrong_broker: dict) -> dict[str, list[str]]:
    result = {}
    for mode in CASES:
        image = wrong_broker if mode == "wrong-parent" else broker
        result[mode] = [parent["path"], "--mode", mode, "--broker", image["path"],
                        "--broker-cdhash", image["cdhash"], "--parent-cdhash", parent["cdhash"],
                        "--evidence", str(run_root / ("case-" + mode))]
    return result


def prepare(output_root: Path) -> Path:
    require_platform()
    parent_root = private_directory(output_root)
    run_root = Path(tempfile.mkdtemp(prefix="tls-native-", dir=parent_root)).resolve(strict=True)
    run_root.chmod(0o700)
    environment = os.environ.copy()
    environment.pop("COLOSSUS_TLS_BROKER_PARENT_CDHASH", None)
    environment.pop("CARGO_TARGET_DIR", None)
    task_tmp = run_root / "cargo-tmp"
    task_tmp.mkdir(mode=0o700)
    environment["TMPDIR"] = str(task_tmp)
    supported_toolchain(environment, run_root)
    before = source_digests()
    parent_target = run_root / "target-parent"
    run_tool(["cargo", "build", "--locked", "--manifest-path", str(MANIFEST), "--target-dir", str(parent_target),
              "--example", "native-parent-acceptance", "--features", "macos-https-broker-native-acceptance"],
             environment, run_root / "parent-build.log")
    parent = signed_image(parent_target / "debug/examples/native-parent-acceptance", run_root / "native-parent-acceptance",
                          "com.obscuritylabs.colossus.native-tls-parent-acceptance", environment, run_root)
    bad_hash = wrong_hash(parent["cdhash"])
    images = {}
    for name, seal in (("tls-broker", parent["cdhash"]), ("tls-broker-wrong-parent", bad_hash)):
        target = run_root / ("target-" + name)
        sealed = environment.copy()
        sealed["COLOSSUS_TLS_BROKER_PARENT_CDHASH"] = seal
        run_tool(["cargo", "build", "--locked", "--manifest-path", str(MANIFEST), "--target-dir", str(target),
                  "--bin", "colossus-native-browser-tls-broker", "--features", "macos-https-broker-entry-development"],
                 sealed, run_root / (name + "-build.log"))
        images[name] = signed_image(target / "debug/colossus-native-browser-tls-broker", run_root / name,
                                    "com.obscuritylabs.colossus." + name + ".acceptance", environment, run_root)
    for mode in CASES:
        (run_root / ("case-" + mode)).mkdir(mode=0o700)
    after = source_digests()
    if before != after:
        raise AcceptanceError("fixture source changed during preparation; preserve and prepare again")
    receipt = {"schema_version": 1, "run_root": str(run_root), "parent": parent, "broker": images["tls-broker"],
               "wrong_broker": images["tls-broker-wrong-parent"], "compiled_parent_cdhash": parent["cdhash"],
               "wrong_parent_cdhash": bad_hash, "commands": case_commands(run_root, parent, images["tls-broker"], images["tls-broker-wrong-parent"]),
               "source_digests": after, "production_enabled": False, "native_cases_executed": False}
    path = run_root / "prepared.json"
    write_json(path, receipt)
    return path


def load_prepared(path: Path) -> dict:
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_nlink != 1 or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_size > 1024 * 1024:
        raise AcceptanceError("prepared receipt must be one private bounded file")
    receipt = json.loads(path.read_text())
    fields = {"schema_version", "run_root", "parent", "broker", "wrong_broker", "compiled_parent_cdhash", "wrong_parent_cdhash", "commands", "source_digests", "production_enabled", "native_cases_executed"}
    if set(receipt) != fields or receipt["schema_version"] != 1 or receipt["production_enabled"] is not False or receipt["native_cases_executed"] is not False:
        raise AcceptanceError("prepared receipt has an unsupported contract")
    run_root = private_directory(Path(receipt["run_root"]))
    for name in ("parent", "broker", "wrong_broker"):
        image = receipt[name]
        if set(image) != {"path", "sha256", "cdhash"}:
            raise AcceptanceError("prepared image identity is malformed")
        image_path = Path(image["path"])
        metadata = image_path.lstat()
        if image_path.parent != run_root or not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_nlink != 1 or stat.S_IMODE(metadata.st_mode) != 0o500 or image_path.resolve(strict=True) != image_path or hashlib.sha256(image_path.read_bytes()).hexdigest() != image["sha256"] or not re.fullmatch(r"[0-9a-f]{40}", image["cdhash"]):
            raise AcceptanceError("prepared image changed or is not a private immutable fixture")
    expected = case_commands(run_root, receipt["parent"], receipt["broker"], receipt["wrong_broker"])
    if receipt["commands"] != expected or receipt["compiled_parent_cdhash"] != receipt["parent"]["cdhash"] or receipt["wrong_parent_cdhash"] != wrong_hash(receipt["parent"]["cdhash"]):
        raise AcceptanceError("prepared launch commands or independent seals changed")
    return receipt


def verify_result(mode: str, result: dict) -> None:
    common = ("accepted", "broker_exit_observed", "listener_v4_closed", "listener_v6_closed", "stdout_empty")
    if any(result.get(field) is not True for field in common) or result.get("forced") is not False or result.get("full_helper_cleanup_proven") is not False or result.get("production_enabled") is not False:
        raise AcceptanceError("native case did not preserve its exact evidence boundary")
    if mode == "wrong-parent":
        if result.get("broker_exit_code") != 11 or result.get("broker_reaped") is not True or result.get("normal_tls") is not False or result.get("shutdown_ack") is not False:
            raise AcceptanceError("wrong-parent case did not reject before availability")
    else:
        if any(result.get(field) is not True for field in ("normal_tls", "leaf_pin_matches", "upstream_observed", "proxy_credential_stripped", "tracked_socket_closed")):
            raise AcceptanceError("native TLS or held-socket evidence is incomplete")
        if mode == "parent-crash":
            if result.get("broker_exit_code") not in (20, 24) or result.get("broker_reaped") is not False or result.get("parent_reaped") is not True or result.get("parent_crashed_without_drop") is not True or result.get("parent_exit_code") != 73 or result.get("shutdown_ack") is not False:
                raise AcceptanceError("crash case lost exact parent/observer evidence")
        elif result.get("broker_reaped") is not True or result.get("broker_exit_code") != (0 if mode == "positive" else 20) or result.get("shutdown_ack") is not (mode == "positive"):
            raise AcceptanceError("native shutdown category was not established")


def run_prepared(path: Path, case: str) -> None:
    require_platform()
    receipt = load_prepared(path)
    run_root = Path(receipt["run_root"])
    environment = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "TMPDIR": str(run_root / "cargo-tmp")}
    modes = CASES if case == "all" else (case,)
    for mode in modes:
        evidence = run_root / ("case-" + mode)
        private_directory(evidence)
        if any(evidence.iterdir()):
            raise AcceptanceError("native evidence case is already allocated; prepare a fresh run")
        command = receipt["commands"][mode]
        # The operator authorized this explicit launch. subprocess owns only its
        # direct parent fixture; descendant duties belong to the native protocol.
        run_tool(command, environment, run_root / (mode + "-native.log"), 120)
        result_path = evidence / "result.json"
        metadata = result_path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_uid != os.geteuid() or metadata.st_nlink != 1 or metadata.st_size > 8192:
            raise AcceptanceError("native result is not one private bounded receipt")
        result = json.loads(result_path.read_text())
        verify_result(mode, result)
        print(mode + ": exact native assertions passed; whole-helper/production claims remain false", flush=True)


def main() -> int:
    args = parser().parse_args()
    try:
        if args.run:
            if args.prepared is None or args.output_root is not None:
                raise AcceptanceError("--run requires --prepared and no output directory")
            run_prepared(args.prepared, args.case)
        else:
            if args.output_root is None or args.prepared is not None or args.case != "all":
                raise AcceptanceError("preparation requires --output-root and no prepared/case selectors")
            receipt = prepare(args.output_root)
            print("Prepared signed native fixture without launching it: " + str(receipt), flush=True)
    except (AcceptanceError, OSError, subprocess.SubprocessError, json.JSONDecodeError, UnicodeError) as error:
        print("Native TLS fixture stopped; allocation retained: " + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
