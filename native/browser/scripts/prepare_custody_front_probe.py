#!/usr/bin/env python3
"""Compile/sign an isolated no-CEF pre-body fixture by default.

This is a standalone development diagnostic, never a completed owned Chromium
artifact receipt or native custody acceptance report. Explicit --run executes
only its five finite native front probes; Root/operator owns those launches.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "native/browser"
MARKERS = tuple(".colossus-custody-probe-" + kind for kind in ("read", "write", "rename", "unlink", "link"))
SOURCE_NAMES = ("front_loader.c", "fixed_policy.c", "policy_transition.c", "custody_policy.c", "custody_policy.h", "custody_probe.c", "custody_probe.h", "image_verify.c", "image_verify.h")
SOURCE_KEYS = frozenset("native/browser/process-broker/" + name for name in SOURCE_NAMES)
CONTROLLER_SOURCE = "native/browser/process-broker/tests/custody_front_controller.c"

def command(values: list[str]) -> None:
    subprocess.run(values, check=True, timeout=120, capture_output=True, text=True)

def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def code_hash(path: Path) -> str:
    value = subprocess.run(["/usr/bin/codesign", "-dv", "--verbose=4", str(path)], check=True, capture_output=True, text=True, timeout=30)
    found = re.search(r"^CDHash=([0-9a-f]{40})$", value.stderr, re.MULTILINE)
    if not found:
        raise ValueError("signed fixture lacks its exact CDHash")
    return found[1]

def prepare(destination: Path) -> Path:
    if os.getuid() == 0 or os.getuid() != os.geteuid() or destination.exists():
        raise ValueError("fixture requires a new explicit unprivileged destination")
    destination.mkdir(mode=0o700)
    destination = destination.resolve(strict=True)
    app = destination / "Colossus Custody Probe.app"
    for path in (app / "Contents/MacOS", app / "Contents/Resources", app / "Contents/Frameworks", destination / "profile", destination / "custody", destination / "build"):
        path.mkdir(parents=True, mode=0o700, exist_ok=True)
    # Custody is a fresh sibling of the separately allocated browser root.
    allocation = destination / "allocation"
    allocation.mkdir(mode=0o700)
    app.rename(allocation / app.name)
    app = allocation / app.name
    (allocation / "profile").mkdir(mode=0o700)
    binding = f"COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n42\n12345\n{allocation}\n{app}\ncom.obscuritylabs.colossus.custody-probe\n{allocation / 'profile'}\n{destination / 'custody'}\n/Users/fixture\n".encode()
    policy = app / "Contents/Resources/colossus-network-envelope.policy"
    policy.write_bytes(binding)
    policy.chmod(0o400)
    for marker in MARKERS:
        path = destination / "custody" / marker
        path.write_bytes(b"colossus public custody probe v1\n")
        path.chmod(0o600)
    generated = destination / "build/network_policies.generated.h"
    command(["python3", "-B", str(SOURCE / "scripts/mac_network_policies.py"), "--source", str(SOURCE / "mac/sandbox"), "--output", str(generated)])
    common = [str(SOURCE / "process-broker" / name) for name in ("front_loader.c", "fixed_policy.c", "policy_transition.c", "custody_policy.c", "custody_probe.c", "image_verify.c")]
    common += [str(SOURCE / "mac/network_envelope.cc")]
    body_source = destination / "build/sentinel.c"
    body_source.write_text('__attribute__((visibility("default"))) int colossus_owned_body_main(int a,char** b,unsigned p,const unsigned char* n,unsigned char r){(void)a;(void)b;(void)p;(void)n;(void)r;return 86;}\n')
    body = destination / "build/Probe Body.dylib"
    command(["/usr/bin/clang", "-dynamiclib", "-std=c11", "-Wall", "-Wextra", "-Werror", str(body_source), "-o", str(body)])
    fronts: list[tuple[int, Path]] = [(0, app / "Contents/MacOS/colossus-native-browser-host")]
    for role, suffix in ((1, " (Renderer)"), (2, " (GPU)"), (3, ""), (4, "")):
        name = f"Colossus Browser Helper{suffix}"
        bundle = app / "Contents/Frameworks" / (name + ".app")
        target = bundle / "Contents/MacOS" / name
        if not target.exists():
            target.parent.mkdir(parents=True, mode=0o700)
            (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": f"com.obscuritylabs.colossus.custody-probe.helper{role}", "CFBundleExecutable": name, "CFBundlePackageType": "APPL"}))
            command(["/usr/bin/clang", "-x", "c", "-std=c11", "-Wall", "-Wextra", "-Werror", "-I" + str(generated.parent), "-I" + str(SOURCE / "mac"), *common, "-lsandbox", "-o", str(target)])
        fronts.append((role, target))
    (app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.obscuritylabs.colossus.custody-probe", "CFBundleExecutable": fronts[0][1].name, "CFBundlePackageType": "APPL"}))
    command(["/usr/bin/clang", "-x", "c", "-std=c11", "-Wall", "-Wextra", "-Werror", "-DCOLOSSUS_OWNED_MAIN_FRONT=1", "-I" + str(generated.parent), "-I" + str(SOURCE / "mac"), *common, "-lsandbox", "-o", str(fronts[0][1])])
    for _, front in fronts:
        sibling = front.with_name(front.name + " Body.dylib")
        if not sibling.exists():
            sibling.write_bytes(body.read_bytes())
            sibling.chmod(0o500)
            command(["/usr/bin/codesign", "--force", "--sign", "-", str(sibling)])
    for bundle in sorted((app / "Contents/Frameworks").glob("*.app")):
        command(["/usr/bin/codesign", "--force", "--sign", "-", str(bundle)])
    command(["/usr/bin/codesign", "--force", "--sign", "-", str(app)])
    command(["/usr/bin/codesign", "--verify", "--strict", "--deep", str(app)])
    controller = destination / "colossus-custody-front-controller"
    command(["/usr/bin/clang", "-std=c11", "-Wall", "-Wextra", "-Werror", str(SOURCE / "process-broker/tests/custody_front_controller.c"), "-lsandbox", "-framework", "Security", "-framework", "CoreFoundation", "-o", str(controller)])
    command(["/usr/bin/codesign", "--force", "--sign", "-", str(controller)])
    source_paths = [SOURCE / "process-broker" / name for name in SOURCE_NAMES]
    receipt = {"schema_version": 1, "purpose": "colossus-native-front-custody-probe", "production_accepted": False, "cef_artifact_receipt": False,
               "package": str(app), "policy_sha256": digest(policy), "controller_sha256": digest(controller), "controller_cdhash": code_hash(controller), "controller_source_sha256": digest(ROOT / CONTROLLER_SOURCE),
               "native_source_sha256": {str(path.relative_to(ROOT)): digest(path) for path in source_paths}, "roles": []}
    for role, front in fronts:
        sibling = front.with_name(front.name + " Body.dylib")
        receipt["roles"].append({"role": role, "front_sha256": digest(front), "front_cdhash": code_hash(front), "body_sha256": digest(sibling),
            "command": [str(controller), str(front), str(role), code_hash(front), digest(sibling), digest(policy), str(destination / "custody" / MARKERS[0])]})
    target = destination / "prepared.json"
    target.write_text(json.dumps(receipt, indent=2) + "\n")
    target.chmod(0o600)
    return target

def marker_generations(destination: Path) -> list[tuple[int, ...]]:
    root = destination / "custody"
    state = root.lstat()
    if root.resolve(strict=True) != root or not root.is_dir() or state.st_uid != os.geteuid() or state.st_mode & 0o7777 != 0o700:
        raise ValueError("unsafe fixture custody parent")
    output = []
    for name in MARKERS:
        path = root / name
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            before = os.fstat(descriptor)
            observed = os.read(descriptor, 1024)
            after = path.lstat()
            if not path.is_file() or before.st_uid != os.geteuid() or before.st_nlink != 1 or before.st_mode & 0o7777 != 0o600 or observed != b"colossus public custody probe v1\n" or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                raise ValueError("fixture marker generation or public bytes changed")
            output.append((before.st_dev, before.st_ino, before.st_uid, before.st_mode, before.st_nlink, before.st_size, before.st_mtime_ns, before.st_ctime_ns))
        finally:
            os.close(descriptor)
    for path in (root / ".colossus-custody-probe-create", destination / "allocation/profile/.colossus-custody-probe-alias"):
        if path.exists() or path.is_symlink():
            raise ValueError("fixture denial destination unexpectedly exists")
    return output

def run_prepared(destination: Path, selected: str) -> Path:
    destination = destination.resolve(strict=True)
    prepared_path = destination / "prepared.json"
    prepared = json.loads(prepared_path.read_text())
    if prepared.get("schema_version") != 1 or prepared.get("purpose") != "colossus-native-front-custody-probe" or prepared.get("production_accepted") is not False or prepared.get("cef_artifact_receipt") is not False:
        raise ValueError("only the fixed standalone no-CEF prepared fixture may run")
    app = destination / "allocation/Colossus Custody Probe.app"
    controller = destination / "colossus-custody-front-controller"
    policy = app / "Contents/Resources/colossus-network-envelope.policy"
    if prepared["package"] != str(app) or digest(controller) != prepared["controller_sha256"] or code_hash(controller) != prepared["controller_cdhash"] or digest(policy) != prepared["policy_sha256"]:
        raise ValueError("prepared package/controller/policy changed")
    command(["/usr/bin/codesign", "--verify", "--strict", "--deep", str(app)])
    command(["/usr/bin/codesign", "--verify", "--strict", str(controller)])
    verify_source_receipt(prepared)
    for relative, expected in prepared["native_source_sha256"].items():
        path = ROOT / relative
        if not path.is_relative_to(SOURCE) or digest(path) != expected:
            raise ValueError("reviewed native source changed")
    role_ids = {"main": 0, "renderer": 1, "gpu": 2, "network": 3, "utility": 4}
    roles = range(5) if selected == "all" else (role_ids[selected],)
    if [item["role"] for item in prepared["roles"]] != list(range(5)):
        raise ValueError("fixture requires the exact ordered five roles")
    before = marker_generations(destination)
    result = {"schema_version": 1, "purpose": "colossus-native-front-custody-probe-result", "prepared_sha256": digest(prepared_path), "production_accepted": False, "cef_artifact_receipt": False, "roles": []}
    target = destination / "native-result.json"
    for role in roles:
        item = prepared["roles"][role]
        if role == 0:
            front = app / "Contents/MacOS/colossus-native-browser-host"
        else:
            suffix = {1: " (Renderer)", 2: " (GPU)", 3: "", 4: ""}[role]
            name = "Colossus Browser Helper" + suffix
            front = app / "Contents/Frameworks" / (name + ".app") / "Contents/MacOS" / name
        body = front.with_name(front.name + " Body.dylib")
        if digest(front) != item["front_sha256"] or code_hash(front) != item["front_cdhash"] or digest(body) != item["body_sha256"]:
            raise ValueError("exact signed role images changed")
        # Derive this closed command ourselves; never execute a manifest-selected
        # command, browser argument, external probe path or arbitrary filter.
        args = [str(controller), str(front), str(role), item["front_cdhash"], item["body_sha256"], prepared["policy_sha256"], str(destination / "custody" / MARKERS[0])]
        process = subprocess.run(args, capture_output=True, text=True, timeout=15, check=False)
        receipt = json.loads(process.stdout) if process.returncode == 0 and len(process.stdout) <= 4096 else None
        accepted = process.returncode == 0 and accepted_receipt(receipt, role) and marker_generations(destination) == before
        result["roles"].append({"role": role, "front_cdhash": item["front_cdhash"], "front_sha256": item["front_sha256"], "body_sha256": item["body_sha256"], "exit_code": process.returncode, "accepted": accepted, "receipt": receipt, "stderr": process.stderr[:4096]})
        target.write_text(json.dumps(result, indent=2) + "\n")
        target.chmod(0o600)
        if not accepted:
            raise ValueError("native role denial/reaping failed; fixture and obligations retained")
    return target

def verify_source_receipt(prepared: dict) -> None:
    values = prepared.get("native_source_sha256")
    if not isinstance(values, dict) or set(values) != SOURCE_KEYS or prepared.get("controller_source_sha256") != digest(ROOT / CONTROLLER_SOURCE):
        raise ValueError("fixture requires exact finite native source/controller bindings")
    for relative, expected in values.items():
        if digest(ROOT / relative) != expected:
            raise ValueError("reviewed native source changed")

def accepted_receipt(receipt: object, role: int) -> bool:
    expected = {"schema_version": 1, "role": role, "pre_body_denials": 511, "direct_child_reaped": True, "exit_code": 0, "cef_loaded": False, "production_accepted": False}
    return isinstance(receipt, dict) and set(receipt) == set(expected) and all(type(receipt[key]) is type(value) and receipt[key] == value for key, value in expected.items())

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--run", action="store_true", help="Root/operator-only: run the existing signed finite fixture; default compiles/signs only")
    parser.add_argument("--role", choices=("all", "main", "renderer", "gpu", "network", "utility"), default="all")
    options = parser.parse_args()
    try:
        if not options.run and options.role != "all":
            raise ValueError("a role is selectable only when explicitly running a prepared fixture")
        print(run_prepared(options.destination.absolute(), options.role) if options.run else prepare(options.destination.absolute()))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("Custody front preparation failed: " + str(error))
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
