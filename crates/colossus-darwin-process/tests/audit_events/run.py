"""Scoped source fixture; received Mach rights stay unclassified and unused."""
from pathlib import Path
import json
import os
import plistlib
import re
import select
import socket
import subprocess
import sys
import tempfile
import time
import uuid

os.umask(0o077)
parent = Path(sys.argv[1]).resolve()
probe = Path(sys.argv[2]).resolve()
rust_producer = Path(sys.argv[3]).resolve()
crash = sys.argv[4] == "crash"
evidence = parent / "receipt.json"
if evidence.exists():
    raise SystemExit("fresh evidence destination required")
nonce = uuid.uuid4().hex
stage = parent / ("pr290-audit-keeper-" + nonce)
stage.mkdir(mode=0o700)
report = {"privileged_installation": False, "audit_policy_changed": False,
          "production_containment": False, "producer_crash": crash}
jobs = []
observer = None

def job(role, args, fresh_session):
    label = "com.colossus.development.audit-" + role + "-" + nonce
    stdout, stderr = stage / (role + ".stdout"), stage / (role + ".stderr")
    for path in (stdout, stderr):
        os.close(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600))
    plist = stage / (role + ".plist")
    program = [str(probe)] + args
    if role == "producer":
        program = [str(rust_producer), "--exact", "fixture_producer_entrypoint", "--ignored", "--nocapture"]
    data = {"Label": label, "ProgramArguments": program,
            "RunAtLoad": True, "KeepAlive": False, "SessionCreate": fresh_session,
            "StandardOutPath": str(stdout), "StandardErrorPath": str(stderr)}
    if role == "keeper":
        data["MachServices"] = {label: True}
    elif role == "producer":
        data["EnvironmentVariables"] = {"COLOSSUS_NATIVE_AUDIT_FIXTURE_SOCKET": args[1],
                                        "COLOSSUS_NATIVE_AUDIT_FIXTURE_SERVICE": args[2],
                                        "COLOSSUS_NATIVE_AUDIT_FIXTURE_PROBE": str(probe)}
    with os.fdopen(os.open(plist, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as output:
        plistlib.dump(data, output)
    target = f"gui/{os.getuid()}/{label}"
    jobs.append((role, target, stdout, stderr))
    launched = subprocess.run(["/bin/launchctl", "bootstrap", f"gui/{os.getuid()}", str(plist)],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
    report[role + "_bootstrap_exit"] = launched.returncode
    if launched.returncode:
        raise RuntimeError(role + " bootstrap failed")
    return label, stdout

try:
    with tempfile.TemporaryDirectory(prefix="cbs-", dir="/private/tmp") as sockets:
        keeper_socket = str(Path(sockets) / "keeper.sock")
        observer_socket = str(Path(sockets) / "scope.sock")
        mach_service = "com.colossus.development.audit-keeper-" + nonce
        _, keeper_stdout = job("keeper", ["keeper", mach_service, keeper_socket], False)
        deadline = time.monotonic() + 5
        while "KEEPER_READY\n" not in keeper_stdout.read_text():
            if time.monotonic() >= deadline:
                raise RuntimeError("owned keeper startup deadline")
            time.sleep(0.02)
        observer_args = [str(probe), "observer-rust-crash" if crash else "observer", observer_socket]
        if crash:
            observer_args.append(str(rust_producer))
        observer = subprocess.Popen(observer_args,
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if not select.select([observer.stdout], [], [], 5)[0]:
            raise RuntimeError("owned observer startup deadline")
        first = observer.stdout.readline()
        if first != b"OBSERVER_READY\n":
            raise RuntimeError("owned observer startup failed")
        job("producer", ["producer", observer_socket, mach_service], True)
        out, err = observer.communicate(timeout=12)
        report["observer_exit"] = observer.returncode
        report["observer_receipt"] = (first + out).decode(errors="replace")[:8192]
        report["observer_diagnostic"] = err.decode(errors="replace")[:1024]
        retained = keeper_stdout.read_text()
        report["keeper_before_release"] = retained[:4096]
        complete = observer.returncode == 0 and "session_end=1" in report["observer_receipt"]
        report["kernel_end_observed"] = complete
        target = re.search(r"observer_asid=(\d+) producer_asid=(\d+)", report["observer_receipt"])
        held = re.search(r"claimed_target_asid=(\d+) keeper_outside_asid=(\d+)", retained)
        report["claimed_right_outside_until_end_unclassified"] = bool(complete and target and held and
            target[2] == held[1] and target[1] == held[2] and target[1] != target[2] and
            "genuine_mach_peer=true received_right_unclassified=true" in retained and
            "unclassified_received_right_released" not in retained)
        if not report["claimed_right_outside_until_end_unclassified"]:
            raise RuntimeError("outside observation fixture remains unknown")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as control:
            control.settimeout(2)
            control.connect(keeper_socket)
            control.sendall(b"D")
        deadline = time.monotonic() + 2
        while "unclassified_received_right_released=true\n" not in keeper_stdout.read_text():
            if time.monotonic() >= deadline:
                raise RuntimeError("owned keeper release deadline")
            time.sleep(0.02)
except Exception as error:
    report["failure_category"] = type(error).__name__
    report["failure_detail"] = str(error)
finally:
    if observer is not None and observer.poll() is None:
        observer.kill()
        observer.wait(timeout=5)
    for role, target, stdout, stderr in reversed(jobs):
        report[role + "_owned_job_absent"] = False
        try:
            subprocess.run(["/bin/launchctl", "bootout", target],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            query = subprocess.run(["/bin/launchctl", "print", target],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
            label = target.rsplit("/", 1)[1]
            report[role + "_owned_job_absent"] = (query.returncode == 113 and
                f'Could not find service "{label}"'.encode() in query.stderr)
        except subprocess.TimeoutExpired:
            report.setdefault("failure_category", "OwnedJobCleanupTimeout")
        report[role + "_receipt"] = stdout.read_text()[:4096]
        report[role + "_diagnostic"] = stderr.read_text()[:1024]
with os.fdopen(os.open(evidence, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as output:
    json.dump(report, output, indent=2)
    output.write("\n")
print(json.dumps(report, indent=2))
if ("failure_category" in report or not report.get("keeper_owned_job_absent") or
        not report.get("producer_owned_job_absent") or
        "RUST_KERNEL_SOURCE_EXPORTED" not in report.get("producer_receipt", "") or
        "RUST_CURRENT_OWNER_PREVENTS_END=true" not in report.get("producer_receipt", "") or
        ("test result: ok." not in report.get("producer_receipt", "") if not crash else
         "producer_crash_signal=0 rust_exporter_crash=true" not in report.get("observer_receipt", ""))):
    raise SystemExit(1)
