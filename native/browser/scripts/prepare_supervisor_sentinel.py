#!/usr/bin/env python3
"""Build/sign the finite no-CEF supervisor fixture; never launch by default.

The operator-only --run accepts only cancel, EOF and actual Core process::exit.
It cannot establish full CEF, a whole process domain, or production availability.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

import prepare_custody_front_probe as fronts

ROOT = fronts.ROOT
FIXTURE = ROOT / "native/browser/process-broker/fixture/Cargo.toml"
BROKER = ROOT / "native/browser/process-broker/Cargo.toml"
CASES = ("cancel", "eof", "parent-crash", "parent-crash-output-loss")

def retire_front_probe_markers(destination: Path) -> None:
    """Remove only the reviewed disposable markers before the real creator owns them."""
    before = fronts.marker_generations(destination)
    root = destination / "custody"
    for name, expected in zip(fronts.MARKERS, before, strict=True):
        marker = root / name
        state = marker.lstat()
        observed = (state.st_dev, state.st_ino, state.st_uid, state.st_mode, state.st_nlink,
                    state.st_size, state.st_mtime_ns, state.st_ctime_ns)
        if observed != expected:
            raise ValueError("disposable front marker generation changed before removal")
        marker.unlink()
    if any((root / name).exists() or (root / name).is_symlink() for name in fronts.MARKERS):
        raise ValueError("real creator marker namespace is not empty")

def verify_result(case: str, result: dict) -> None:
    crash = case in ("parent-crash", "parent-crash-output-loss")
    required = {
        "case", "accepted", "core_reaped", "core_crashed_without_drop", "core_exit_code",
        "creator_exit_observed", "keeper_exit_observed", "creator_exit_code", "keeper_exit_code",
        "creator_exit_signal", "keeper_exit_signal",
        "creator_reaped_by_core", "keeper_reaped_by_core", "known_children_closed", "helpers_reaped",
        "output_eof", "forced", "full_helper_cleanup_proven", "production_enabled",
    }
    if (set(result) != required or case not in CASES or result.get("case") != case or
            result.get("accepted") is not True or result.get("core_reaped") is not True or
            result.get("core_crashed_without_drop") is not crash or
            result.get("core_exit_code") != (73 if crash else 0) or
            result.get("creator_exit_code") not in (0, 20) or result.get("keeper_exit_code") not in (0, 20) or
            result.get("creator_exit_signal") is not None or result.get("keeper_exit_signal") is not None or
            result.get("known_children_closed") is not True or result.get("helpers_reaped") != 4 or
            result.get("output_eof") is not True or result.get("forced") is not False or
            result.get("production_enabled") is not False or result.get("full_helper_cleanup_proven") is not False or
            (crash and (result.get("creator_exit_observed") is not True or result.get("keeper_exit_observed") is not True)) or
            result.get("creator_reaped_by_core") is not (not crash) or
            result.get("keeper_reaped_by_core") is not (not crash)):
        raise ValueError("known supervisor cleanup remains unaccepted")

def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def tool(args: list[str], environment: dict[str, str], output: Path, timeout: int = 1800) -> None:
    with output.open("xb") as log:
        os.fchmod(log.fileno(), 0o600)
        result = subprocess.run(args, cwd=ROOT, env=environment, stdin=subprocess.DEVNULL,
                                stdout=log, stderr=subprocess.STDOUT, timeout=timeout, check=False)
    if result.returncode:
        raise ValueError("fixture tool failed; inspect retained " + str(output))

def sign(path: Path, environment: dict[str, str], output: Path) -> None:
    tool(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", str(path)], environment, output)
    subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(path)], check=True, timeout=30, capture_output=True)

def sources() -> dict[str, str]:
    paths = set()
    for base in (ROOT / "native/browser/process-broker", ROOT / "crates/colossus-darwin-process"):
        for pattern in ("*.toml", "*.lock", "*.c", "*.h", "*.cc", "build.rs", "src/**/*.rs", "tests/*.cc", "fixture/*.toml", "fixture/*.lock", "fixture/*.c", "fixture/build.rs", "fixture/src/**/*.rs"):
            paths.update(base.glob(pattern))
    paths.add(Path(__file__).resolve())
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(paths)}

def prepare(destination: Path) -> Path:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    # The ephemeral fixture private key stays only in this preparer's memory.
    # It is never a browser resource, input frame, environment entry or file.
    signing_key = Ed25519PrivateKey.generate()
    key = signing_key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex()
    fronts.prepare(destination)
    destination = destination.resolve(strict=True)
    # The source-only front probe compiler prepares public markers itself. The
    # real independent creator must create its own files with create_new(true).
    # No marker may be adopted or overwritten by production custody ownership.
    retire_front_probe_markers(destination)
    (destination / "prepared.json").rename(destination / "front-preparation.json")
    environment = os.environ.copy()
    for name in ("COLOSSUS_CEF_ROOT", "COLOSSUS_CEF_NATIVE_LIB_DIR", "COLOSSUS_MAC_PROCESS_PARENT_CDHASH", "CARGO_TARGET_DIR"):
        environment.pop(name, None)
    environment["CARGO_INCREMENTAL"] = "0"
    environment["COLOSSUS_PROCESS_FIXTURE_ROOT"] = str(destination)
    environment["COLOSSUS_PROCESS_FIXTURE_PUBLIC_KEY"] = key
    environment["CARGO_TARGET_DIR"] = str(ROOT / ".local/supervisor-sentinel-fixture-build")
    app = destination / "allocation/Colossus Custody Probe.app"
    body = destination / "build/Supervisor Sentinel.dylib"
    tool(["/usr/bin/clang++", "-std=c++20", "-Wall", "-Wextra", "-Werror", "-Wexit-time-destructors", "-dynamiclib",
          str(ROOT / "native/browser/process-broker/tests/supervisor_sentinel.cc"),
          str(ROOT / "native/browser/process-broker/client.cc"), "-o", str(body)], environment, destination / "sentinel-build.log")
    names = [app / "Contents/MacOS/colossus-native-browser-host"]
    for role in range(1, 5):
        suffix = {1:" (Renderer)", 2:" (GPU)", 3:"", 4:""}[role]
        name = "Colossus Browser Helper" + suffix
        names.append(app / "Contents/Frameworks" / (name + ".app") / "Contents/MacOS" / name)
    for index, front in enumerate(dict.fromkeys(names)):
        target = front.with_name(front.name + " Body.dylib")
        target.chmod(0o700)
        shutil.copyfile(body, target); target.chmod(0o500)
        sign(target, environment, destination / f"sentinel-role-body-{index}-sign.log")
    tool(["cargo", "build", "--locked", "--offline", "--manifest-path", str(FIXTURE), "--features", "native-acceptance", "--bin", "controller"], environment, destination / "controller-build.log")
    controller = destination / "controller"
    shutil.copyfile(Path(environment["CARGO_TARGET_DIR"]) / "debug/controller", controller); controller.chmod(0o500)
    sign(controller, environment, destination / "controller-sign.log")
    environment["COLOSSUS_PROCESS_FIXTURE_CONTROLLER_CDHASH"] = fronts.code_hash(controller)
    tool(["cargo", "build", "--locked", "--offline", "--manifest-path", str(FIXTURE), "--features", "core-entry", "--bin", "core"], environment, destination / "core-build.log")
    core = destination / "core"
    shutil.copyfile(Path(environment["CARGO_TARGET_DIR"]) / "debug/core", core); core.chmod(0o500)
    sign(core, environment, destination / "core-sign.log")
    environment["COLOSSUS_MAC_PROCESS_PARENT_CDHASH"] = fronts.code_hash(core)
    environment["CARGO_TARGET_DIR"] = str(ROOT / ".local/supervisor-sentinel-broker-build")
    tool(["cargo", "build", "--locked", "--offline", "--manifest-path", str(BROKER), "--features", "macos-process-entry-development", "--bin", "colossus-mac-browser-process-broker"], environment, destination / "broker-build.log")
    broker = app / "Contents/MacOS/colossus-mac-browser-process-broker"
    shutil.copyfile(Path(environment["CARGO_TARGET_DIR"]) / "debug/colossus-mac-browser-process-broker", broker); broker.chmod(0o500)
    sign(broker, environment, destination / "broker-sign.log")
    for index, bundle in enumerate(sorted((app / "Contents/Frameworks").glob("*.app"))):
        sign(bundle, environment, destination / f"supervisor-helper-{index}-sign.log")
    sign(app, environment, destination / "supervisor-app-sign.log")
    subprocess.run(["/usr/bin/codesign", "--verify", "--strict", "--deep", str(app)], check=True, timeout=30, capture_output=True)
    profile = destination / "allocation/profile"
    for name in ("home", "config", "data", "tmp"):
        (profile / name).mkdir(mode=0o700)
    images = [{"role": role, "executable": str(front), "cdhash": list(bytes.fromhex(fronts.code_hash(front))),
               "body_sha256": list(bytes.fromhex(digest(front.with_name(front.name + " Body.dylib"))))} for role, front in enumerate(names)]
    env = [f"HOME={profile / 'home'}", f"XDG_CONFIG_HOME={profile / 'config'}", f"XDG_DATA_HOME={profile / 'data'}",
           f"TMPDIR={profile / 'tmp'}", f"MAC_CHROMIUM_TMPDIR={profile / 'tmp'}", "PATH=/usr/bin:/bin", "LANG=en_US.UTF-8", "MallocNanoZone=0"]
    files = [controller, core, *[path for path in app.rglob("*") if path.is_file()]]
    payload = {"schema_version":1, "purpose":"colossus-known-supervisor-sentinel-v1", "production_accepted":False,
               "controller_cdhash":list(bytes.fromhex(fronts.code_hash(controller))), "core_cdhash":list(bytes.fromhex(fronts.code_hash(core))),
               "broker_cdhash":list(bytes.fromhex(fronts.code_hash(broker))),
               "setup":{"nonce":list(os.urandom(16)), "policy_sha256":list(bytes.fromhex(digest(app / "Contents/Resources/colossus-network-envelope.policy"))),
                        "images":images, "environment":env},
               "files":[{"path":str(path.relative_to(destination)), "sha256":list(bytes.fromhex(digest(path)))} for path in sorted(set(files))]}
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    inventory = destination / "inventory.json"
    inventory.write_text(json.dumps({"payload":list(encoded), "signature":list(signing_key.sign(encoded))}, separators=(",", ":")) + "\n"); inventory.chmod(0o400)
    prepared = destination / "prepared.json"
    prepared.write_text(json.dumps({"schema_version":1, "purpose":"colossus-known-supervisor-sentinel-prepared-v1", "root":str(destination),
        "inventory_sha256":digest(inventory), "public_key":key, "source_sha256":sources(), "native_launched":False,
        "full_helper_cleanup_proven":False, "production_enabled":False,
        "commands":{case:[str(controller), case] for case in CASES}}, indent=2) + "\n"); prepared.chmod(0o600)
    return prepared

def run(prepared: Path, case: str) -> Path:
    prepared = prepared.resolve(strict=True)
    value = json.loads(prepared.read_text())
    root = prepared.parent
    if value.get("purpose") != "colossus-known-supervisor-sentinel-prepared-v1" or value.get("root") != str(root) or value.get("schema_version") != 1 or value.get("production_enabled") is not False or value.get("full_helper_cleanup_proven") is not False:
        raise ValueError("only the exact finite prepared supervisor fixture may run")
    if digest(root / "inventory.json") != value["inventory_sha256"] or sources() != value["source_sha256"]:
        raise ValueError("compiled inventory or reviewed source changed")
    selected = CASES if case == "all" else (case,)
    for current in selected:
        # Derive fixed image/case ourselves; never execute an inventory command.
        command = [str(root / "controller"), current]
        tool(command, os.environ.copy(), root / f"{current}-operator.log", timeout=50)
        result = json.loads((root / f"case-{current}/result.json").read_text())
        verify_result(current, result)
    return root

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument("--destination", type=Path, help="new private preparation path; no native launch")
    actions.add_argument("--run-prepared", type=Path, help="operator-only actual native launch from existing prepared.json")
    parser.add_argument("--case", choices=(*CASES, "all"), default="all")
    options = parser.parse_args()
    try:
        if options.destination is not None and options.case != "all":
            raise ValueError("case selection requires an explicit prepared run")
        print(prepare(options.destination.absolute()) if options.destination else run(options.run_prepared, options.case))
        return 0
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print("Supervisor fixture failed: " + str(error))
        return 1
if __name__ == "__main__":
    raise SystemExit(main())
