#!/usr/bin/env python3
"""Prepare or collect finite pre-body denial evidence for an existing owned app.

Neither mode launches native processes or changes the signed component. Root or
the operator executes the five emitted controller commands separately. Collection
produces an unsigned denial report for later independently authenticated package
composition; a preparation receipt is never native acceptance or production trust.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import sys

import component
import owned_process_component
import prepare_core_macos_owned as core
import prepare_custody_front_probe as standalone
import run_core_macos_diagnostic as common
import stage_macos

ROOT = common.ROOT
PURPOSE = "colossus-owned-component-custody-preparation"
POLICY_SOURCES = ("custody_policy.c", "custody_policy.h", "fixed_policy.c",
                  "front_loader.c", "policy_transition.c", "custody_probe.c", "custody_probe.h")
SOURCE_KEYS = frozenset("native/browser/process-broker/" + name for name in POLICY_SOURCES)
CONTENT = b"colossus public custody probe v1\n"
DENIALS_NAME = "owned-process-custody-denials.json"
MARKERS = standalone.MARKERS


def private_json(path: Path, limit: int) -> dict:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        before = os.fstat(fd)
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.geteuid() or
                before.st_nlink != 1 or stat.S_IMODE(before.st_mode) & 0o077 or
                before.st_size > limit):
            raise common.DiagnosticError("unsafe private evidence input")
        chunks = bytearray()
        while len(chunks) <= limit:
            part = os.read(fd, limit + 1 - len(chunks))
            if not part:
                break
            chunks.extend(part)
        raw = bytes(chunks)
        after = path.lstat()
        if identity(before) != identity(os.fstat(fd)) or identity(before) != identity(after):
            raise common.DiagnosticError("private evidence input changed")
        value = json.loads(raw)
        if len(raw) != before.st_size or not isinstance(value, dict):
            raise common.DiagnosticError("invalid bounded evidence input")
        return value
    finally:
        os.close(fd)


def identity(value: os.stat_result) -> list[int]:
    return [value.st_dev, value.st_ino, value.st_uid, value.st_mode,
            value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns]


def evidence_path(path: Path) -> Path:
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise common.DiagnosticError("evidence path must be canonical and absolute")
    return path


def role_paths(app: Path) -> list[Path]:
    output = [app / "Contents/MacOS/colossus-native-browser-host"]
    for suffix in core.HELPER_SUFFIXES:
        name = stage_macos.HELPER + suffix
        output.append(app / "Contents/Frameworks" / (name + ".app") / "Contents/MacOS" / name)
    return output


def policy_namespace(raw: bytes, allocation: Path, app: Path) -> Path:
    try:
        text = raw.decode("utf-8")
        fields = text.splitlines()
        session, port = int(fields[1]), int(fields[2])
    except (UnicodeError, ValueError, IndexError) as error:
        raise common.DiagnosticError("invalid fixed network policy") from error
    if (not text.endswith("\n") or len(fields) != 9 or
            fields[0] != "COLOSSUS_MAC_NETWORK_ENVELOPE_V1" or
            not 0 < session < 0xffffffff or str(session) != fields[1] or
            not 0 < port < 65536 or str(port) != fields[2] or
            fields[3] != str(allocation) or fields[4] != str(app) or
            fields[6] != str(allocation / "profile")):
        raise common.DiagnosticError("policy does not bind the exact existing allocation")
    root = Path(fields[7])
    if (not root.is_absolute() or str(root) != fields[7] or root == Path("/") or
            root.resolve(strict=True) != root or root.is_relative_to(allocation) or
            allocation.is_relative_to(root)):
        raise common.DiagnosticError("custody root is not a separate canonical namespace")
    common.private_parent(root)
    return root


def inspect_stage(path: Path, evidence: Path, environment: dict[str, str]) -> tuple[dict, dict, dict, Path]:
    stage = private_json(path, 16 * 1024)
    allocation = common.existing_path(Path(stage["allocation_root"]), directory=True)
    app = common.existing_path(Path(stage["app_bundle"]), directory=True)
    if (stage.get("schema_version") != 1 or stage.get("development_only") is not True or
            stage.get("production_accepted") is not False or not app.is_relative_to(allocation)):
        raise common.DiagnosticError("only an existing owned development stage may be probed")
    descriptor_path = allocation / "owned-process-host.json"
    descriptor = private_json(descriptor_path, 16 * 1024)
    if (common.digest(descriptor_path) != stage["descriptor_sha256"] or
            descriptor.get("purpose") != core.PURPOSE or descriptor.get("production_accepted") is not False or
            descriptor.get("allocation_root") != str(allocation) or descriptor.get("app_bundle") != str(app)):
        raise common.DiagnosticError("original staged descriptor changed")
    manifest_path = app.with_name(app.name + ".browser-component.json")
    manifest = owned_process_component.verify_installed(app, manifest_path)
    if common.digest(manifest_path) != descriptor["manifest_sha256"]:
        raise common.DiagnosticError("original complete component inventory changed")
    sources = manifest["owned_process"]["native_source_sha256"]
    if not SOURCE_KEYS.issubset(sources):
        raise common.DiagnosticError("completed component lacks reviewed policy sources")
    for relative in SOURCE_KEYS:
        if common.digest(ROOT / relative) != sources[relative]:
            raise common.DiagnosticError("current reviewed policy source differs from the real component")
    policy = app / "Contents/Resources" / stage_macos.NETWORK_ENVELOPE_RESOURCE
    if common.digest(policy) != descriptor["policy_sha256"]:
        raise common.DiagnosticError("original signed policy changed")
    root = policy_namespace(policy.read_bytes(), allocation, app)
    if root != Path(stage["broker_root"]):
        raise common.DiagnosticError("original custody root differs from stage")
    expected = [descriptor["front_cdhash"], *descriptor["helper_cdhashes"]]
    if len(expected) != 5:
        raise common.DiagnosticError("exact five native role seals required")
    common.run_logged(["/usr/bin/codesign", "--verify", "--strict", "--deep", str(app)],
                      environment, evidence / "component-signature.log", timeout=30)
    for role, (front, seal) in enumerate(zip(role_paths(app), expected)):
        if common.cdhash(front, environment, evidence, f"role-{role}-signature.log") != seal:
            raise common.DiagnosticError("actual fixed role seal differs from original descriptor")
    return stage, descriptor, manifest, root


def marker_state(root: Path, profile: Path) -> dict[str, list[int]]:
    common.private_parent(root)
    output = {}
    for name in MARKERS:
        path = root / name
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            before = os.fstat(fd)
            if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.geteuid() or
                    before.st_nlink != 1 or stat.S_IMODE(before.st_mode) != 0o600 or
                    before.st_size != len(CONTENT) or os.read(fd, len(CONTENT) + 1) != CONTENT or
                    identity(before) != identity(path.lstat()) or identity(before) != identity(os.fstat(fd))):
                raise common.DiagnosticError("exact plain custody marker generation changed")
            output[name] = identity(before)
        finally:
            os.close(fd)
    for absent in (root / ".colossus-custody-probe-create", profile / ".colossus-custody-probe-alias"):
        if absent.exists() or absent.is_symlink():
            raise common.DiagnosticError("custody denial destination unexpectedly exists")
    return output


def prepare(stage_path: Path, parent: Path) -> Path:
    evidence = common.private_child(common.private_parent(parent), "owned-custody-")
    environment = dict(os.environ)
    stage_path = evidence_path(stage_path)
    stage, descriptor, manifest, root = inspect_stage(stage_path, evidence, environment)
    controller = evidence / "colossus-custody-front-controller"
    source = ROOT / standalone.CONTROLLER_SOURCE
    common.run_logged(["/usr/bin/clang", "-std=c11", "-Wall", "-Wextra", "-Werror", str(source),
                       "-lsandbox", "-framework", "Security", "-framework", "CoreFoundation",
                       "-o", str(controller)], environment, evidence / "controller-build.log", timeout=120)
    common.run_logged(["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none", str(controller)],
                      environment, evidence / "controller-sign.log", timeout=30)
    common.run_logged(["/usr/bin/codesign", "--verify", "--strict", str(controller)],
                      environment, evidence / "controller-verify.log", timeout=30)
    # These fixed plain files are the only additions under the existing broker
    # root. Never replace a creator's active marker, identity or credential file.
    for name in MARKERS:
        common.write_private(root / name, CONTENT)
    generations = marker_state(root, Path(stage["allocation_root"]) / "profile")
    records = []
    for role, front in enumerate(role_paths(Path(stage["app_bundle"]))):
        body = front.with_name(front.name + " Body.dylib")
        seal = descriptor["front_cdhash"] if role == 0 else descriptor["helper_cdhashes"][role - 1]
        records.append({"role": role, "front": str(front), "front_cdhash": seal,
                        "front_sha256": common.digest(front), "body_sha256": common.digest(body),
                        "command": [str(controller), str(front), str(role), seal, common.digest(body),
                                    descriptor["policy_sha256"], str(root / MARKERS[0])],
                        "stdout": f"role-{role}.stdout.json", "stderr": f"role-{role}.stderr.txt",
                        "exit": f"role-{role}.exit.json"})
    prepared = {"schema_version": 1, "purpose": PURPOSE, "production_accepted": False,
                "native_acceptance": False, "cef_loaded": False, "stage_receipt": str(stage_path),
                "stage_sha256": common.digest(stage_path), "component_manifest_sha256": descriptor["manifest_sha256"],
                "descriptor_sha256": stage["descriptor_sha256"], "policy_sha256": descriptor["policy_sha256"],
                "controller_sha256": common.digest(controller),
                "controller_cdhash": common.cdhash(controller, environment, evidence, "controller-signature.log"),
                "controller_source_sha256": common.digest(source),
                "native_source_sha256": {name: manifest["owned_process"]["native_source_sha256"][name]
                                         for name in sorted(SOURCE_KEYS)},
                "marker_generations": generations, "roles": records}
    common.write_json(evidence / "prepared.json", prepared)
    return evidence / "prepared.json"


def validate_results(prepared: dict, rows: list[tuple[dict, dict, bytes, dict]]) -> dict:
    if len(rows) != 5 or len(prepared["roles"]) != 5:
        raise common.DiagnosticError("all five actual role results required")
    roles = []
    for index, (item, receipt, stderr, exit_record) in enumerate(rows):
        if (type(item.get("role")) is not int or item["role"] != index or
                not standalone.accepted_receipt(receipt, index) or stderr or
                set(exit_record) != {"exit_code"} or type(exit_record["exit_code"]) is not int or
                exit_record["exit_code"] != 0):
            raise common.DiagnosticError("actual native pre-body denial or reap result rejected")
        roles.append({"role": index, "cdhash": item["front_cdhash"],
                      **{name: True for name in ("read_denied", "write_denied", "create_denied",
                          "rename_denied", "unlink_denied", "link_denied", "extension_denied",
                          "extension_use_denied", "security_mach_denied", "direct_child_reaped")},
                      "exit_code": 0})
    return {"schema_version": 1, "purpose": "colossus-macos-owned-process-custody-denials",
            "production_accepted": False, "component_manifest_sha256": prepared["component_manifest_sha256"],
            "policy_sha256": prepared["policy_sha256"], "native_source_sha256": prepared["native_source_sha256"],
            "roles": roles}


def collect(prepared_path: Path) -> Path:
    prepared_path = evidence_path(prepared_path)
    evidence = common.private_parent(prepared_path.parent)
    prepared = private_json(prepared_path, 32 * 1024)
    if (prepared.get("schema_version") != 1 or prepared.get("purpose") != PURPOSE or
            prepared.get("production_accepted") is not False or prepared.get("native_acceptance") is not False or
            prepared.get("cef_loaded") is not False or set(prepared["native_source_sha256"]) != SOURCE_KEYS):
        raise common.DiagnosticError("only the finite real-component preparation may be collected")
    stage_path = Path(prepared["stage_receipt"])
    if common.digest(stage_path) != prepared["stage_sha256"]:
        raise common.DiagnosticError("original stage receipt changed")
    stage, descriptor, manifest, root = inspect_stage(stage_path, evidence, dict(os.environ))
    if (prepared["component_manifest_sha256"] != descriptor["manifest_sha256"] or
            prepared["descriptor_sha256"] != stage["descriptor_sha256"] or
            prepared["policy_sha256"] != descriptor["policy_sha256"] or
            prepared["native_source_sha256"] != {name: manifest["owned_process"]["native_source_sha256"][name]
                                                 for name in SOURCE_KEYS} or
            prepared["controller_source_sha256"] != common.digest(ROOT / standalone.CONTROLLER_SOURCE)):
        raise common.DiagnosticError("original actual component or controller sources changed")
    controller = evidence / "colossus-custody-front-controller"
    if (common.digest(controller) != prepared["controller_sha256"] or
            common.cdhash(controller, dict(os.environ), evidence, "controller-final-signature.log") != prepared["controller_cdhash"]):
        raise common.DiagnosticError("prepared controller changed")
    common.run_logged(["/usr/bin/codesign", "--verify", "--strict", str(controller)],
                      dict(os.environ), evidence / "controller-final-verify.log", timeout=30)
    profile = Path(stage["allocation_root"]) / "profile"
    if marker_state(root, profile) != prepared["marker_generations"]:
        raise common.DiagnosticError("own custody marker generations changed")
    rows = []
    for role, (item, front) in enumerate(zip(prepared["roles"], role_paths(Path(stage["app_bundle"])))):
        body = front.with_name(front.name + " Body.dylib")
        expected_seal = descriptor["front_cdhash"] if role == 0 else descriptor["helper_cdhashes"][role - 1]
        expected_command = [str(controller), str(front), str(role), expected_seal, common.digest(body),
                            descriptor["policy_sha256"], str(root / MARKERS[0])]
        if (item["front"] != str(front) or item["front_cdhash"] != expected_seal or
                item["command"] != expected_command or item["front_sha256"] != common.digest(front) or
                item["body_sha256"] != common.digest(body)):
            raise common.DiagnosticError("actual fixed front/body changed")
        if (item["stdout"] != f"role-{role}.stdout.json" or item["stderr"] != f"role-{role}.stderr.txt" or
                item["exit"] != f"role-{role}.exit.json"):
            raise common.DiagnosticError("result paths must be the fixed finite role names")
        stderr = evidence / item["stderr"]
        if stderr.lstat().st_size != 0 or not stat.S_ISREG(stderr.lstat().st_mode) or stderr.is_symlink():
            raise common.DiagnosticError("native probe stderr must be an empty regular file")
        rows.append((item, private_json(evidence / item["stdout"], 4096), b"",
                     private_json(evidence / item["exit"], 4096)))
    proof = validate_results(prepared, rows)
    target = evidence / DENIALS_NAME
    # Recheck every generation before any deletion. Unknown/partial cleanup is
    # retained; never prune the existing broker's other state or the signed app.
    if marker_state(root, profile) != prepared["marker_generations"]:
        raise common.DiagnosticError("own markers changed before cleanup")
    for name in MARKERS:
        (root / name).unlink()
    common.write_json(target, proof)
    return target


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--stage-receipt", type=Path)
    modes.add_argument("--collect", type=Path)
    parser.add_argument("--output-root", type=Path)
    options = parser.parse_args(arguments)
    try:
        if options.stage_receipt:
            if options.output_root is None:
                raise common.DiagnosticError("preparation requires a new private output root")
            result = prepare(options.stage_receipt, options.output_root)
        else:
            if options.output_root is not None:
                raise common.DiagnosticError("collection uses only the fixed prepared evidence directory")
            result = collect(options.collect)
        print(result)
        return 0
    except (common.DiagnosticError, component.ComponentError, OSError, ValueError,
            KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"Owned custody evidence failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
