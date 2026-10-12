"""Pure launcher authority, command selection and retained-evidence regressions."""

from contextlib import contextmanager
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run_core_macos_diagnostic as diagnostic
import stage_macos


@unittest.skipIf(os.name == "nt", "this launcher requires POSIX private files")
class CoreMacosDiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.cef = self.root / "cef"
        self.build = self.root / "build"
        self.personal = self.root / "personal"
        for path in (self.cef, self.build, self.personal):
            path.mkdir()
        self.host = self.root / "colossus-native-browser-host"
        self.host.write_bytes(b"synthetic host fixture")
        self.host.chmod(0o755)
        for name, value in [
            (stage_macos.SYNTHETIC_PROFILE_BUILD, stage_macos.SYNTHETIC_PROFILE_METADATA),
            (stage_macos.NETWORK_ENVELOPE_BUILD, stage_macos.NETWORK_ENVELOPE_METADATA),
        ]:
            (self.build / name).write_text(json.dumps(value))
        self.output = self.root / "evidence-parent"
        self.arguments = ["--cef-root", str(self.cef), "--native-build", str(self.build),
                          "--host-binary", str(self.host), "--output-root", str(self.output)]
        self.commands = []
        self.reservations = []

    def options(self, *extra):
        return diagnostic.parser().parse_args(self.arguments + list(extra))

    @contextmanager
    def reserve(self, port=0, *, ipv6=True):
        self.reservations.append((port, ipv6))
        try:
            yield port or (61024 if ipv6 else 62024)
        finally:
            self.reservations.pop()

    def fake_run(self, arguments, environment, log, **kwargs):
        self.commands.append((arguments, dict(environment), log, kwargs))
        if str(diagnostic.ROOT / "native/browser/scripts/stage_host_macos.py") in arguments:
            self.assertEqual(len(self.reservations), 2)
            self.assertEqual(self.reservations[0], (0, True))
            self.assertFalse(self.reservations[1][1])
            app = Path(arguments[arguments.index("--app") + 1])
            policy = Path(arguments[arguments.index("--macos-network-envelope-policy") + 1])
            executable = app / "Contents/MacOS/colossus-native-browser-host"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"signed front fixture")
            executable.with_name(executable.name + " Body").write_bytes(b"signed body fixture")
            resource = app / "Contents/Resources" / stage_macos.NETWORK_ENVELOPE_RESOURCE
            resource.parent.mkdir(parents=True)
            resource.write_bytes(policy.read_bytes())
            app.with_name(app.name + ".browser-component.json").write_bytes(b"manifest fixture")
        return b""

    @contextmanager
    def boundaries(self):
        with (patch.object(diagnostic, "macos_platform", return_value="macosarm64"),
              patch.object(diagnostic, "require_toolchain"),
              patch.object(diagnostic, "current_audit_session", return_value=123),
              patch.object(diagnostic, "reserve_loopbacks", side_effect=self.reserve),
              patch.object(diagnostic, "personal_home", return_value=self.personal),
              patch.object(diagnostic, "run_logged", side_effect=self.fake_run),
              patch.object(diagnostic, "cdhash", side_effect=["a" * 40, "b" * 40])):
            yield

    def test_detects_only_supported_native_macos_architectures(self):
        for machine, expected in [("arm64", "macosarm64"), ("aarch64", "macosarm64"),
                                  ("x86_64", "macosx64")]:
            with patch.object(diagnostic.sys, "platform", "darwin"), patch.object(diagnostic.platform, "machine", return_value=machine):
                self.assertEqual(diagnostic.macos_platform(), expected)
        for operating_system, machine in [("linux", "x86_64"), ("darwin", "armv7")]:
            with patch.object(diagnostic.sys, "platform", operating_system), patch.object(diagnostic.platform, "machine", return_value=machine):
                with self.assertRaises(diagnostic.DiagnosticError):
                    diagnostic.macos_platform()

    def test_rejects_noncanonical_or_broader_fixture_origins(self):
        self.assertEqual(diagnostic.fixture_port("http://127.0.0.1:60924"), 60924)
        self.assertEqual(diagnostic.fixture_port(None), 0)
        for value in ["http://localhost:60924", "https://127.0.0.1:60924",
                      "http://[::1]:60924", "http://127.0.0.1:80", "http://127.0.0.1:0",
                      "http://user@127.0.0.1:60924", "http://127.0.0.1:60924/",
                      "http://127.0.0.1:60924?x", "http://127.0.0.1:60924#x",
                      "http://127.0.0.1:060924", "http://127.0.0.1:99999"]:
            with self.subTest(value=value), self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.fixture_port(value)

    def test_dispatches_only_the_exact_core_test_and_read_only_identity_example(self):
        command = diagnostic.core_command()
        self.assertEqual(command.count(diagnostic.CORE_TEST), 1)
        self.assertEqual(command[-6:], ["--", "--ignored", "--exact", "--nocapture", "--color", "never"])
        self.assertIn("--lib", command)
        self.assertNotIn("--release", command)
        transfer = diagnostic.core_command("transfers")
        self.assertEqual(transfer.count(diagnostic.TRANSFER_TEST), 1)
        self.assertNotIn(diagnostic.CORE_TEST, transfer)
        with self.assertRaises(KeyError):
            diagnostic.core_command("native_session_ports")
        with patch.object(diagnostic, "run_logged", return_value=b"123\n") as run:
            self.assertEqual(diagnostic.current_audit_session({}, self.root), 123)
        identity = run.call_args.args[0]
        self.assertEqual(identity, ["cargo", "run", "--quiet", "--locked", "-p",
                                   "colossus-darwin-process", "--example", "browser_current_audit_session"])
        self.assertNotIn("--ignored", identity)
        self.assertFalse(any("session_ports" in value or "join" in value for value in command + identity))
        for value in [b"0\n", b"4294967295\n", b"123\nextra\n", b"1" * 33]:
            with patch.object(diagnostic, "run_logged", return_value=value), self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.current_audit_session({}, self.root)

    def test_stages_private_exact_policy_and_desktop_seal_without_running_test(self):
        with self.boundaries(), patch.dict(os.environ, {"UNRELATED_SECRET": "do-not-publish"}):
            self.assertEqual(diagnostic.main(self.arguments + ["--stage-only"]), 0)
        self.assertEqual(len(self.commands), 1)
        command, environment, _, step = self.commands[0]
        self.assertIn("--macos-synthetic-profile-diagnostic", command)
        self.assertNotIn("--macos-owned-profile-development", command)
        self.assertNotIn("--no-sandbox", command)
        self.assertEqual(step["timeout"], diagnostic.BUILD_TIMEOUT)
        evidence = next(self.output.glob("evidence-*"))
        allocation = next(self.output.glob("allocation-*"))
        broker = next(self.output.glob("broker-*"))
        fields = (allocation / "binding.source").read_text().split("\n")
        self.assertEqual(fields, [stage_macos.NETWORK_ENVELOPE_MARKER, "123", "61024",
                                  str(allocation), str(allocation / "Colossus Core Browser.app"),
                                  diagnostic.IDENTIFIER, str(allocation / "profile"), str(broker),
                                  str(self.personal), ""])
        self.assertFalse((allocation / "profile").exists())
        self.assertNotEqual(Path(environment["TMPDIR"]).parent, allocation)
        self.assertEqual(Path(environment["TMPDIR"]).parent, evidence)
        seal = json.loads((evidence / "desktop-compiler-seal.json").read_text())
        self.assertEqual(len(seal), 7)
        self.assertEqual(seal["fixture_origin"], "http://127.0.0.1:62024")
        receipt = json.loads((evidence / "stage-environment.json").read_text())
        self.assertEqual(len(receipt), 6)
        for path in [self.output, evidence, allocation, broker, Path(environment["TMPDIR"])]:
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)
        for path in [allocation / "binding.source", evidence / "desktop-compiler-seal.json",
                     evidence / "stage-environment.json", evidence / "diagnostic-stage.json"]:
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertNotIn("do-not-publish", path.read_text())

    def test_failed_core_run_keeps_exact_allocations_and_receipts(self):
        with self.boundaries():
            evidence, environment = diagnostic.prepare(self.options())
        with patch.object(diagnostic, "prepare", return_value=(evidence, environment)), patch.object(
                diagnostic, "run_logged", side_effect=subprocess.CalledProcessError(1, diagnostic.core_command())) as run:
            self.assertEqual(diagnostic.main(self.arguments), 1)
        self.assertEqual(run.call_args.args[0], diagnostic.core_command())
        self.assertTrue((evidence / "stage-environment.json").is_file())
        self.assertTrue(next(self.output.glob("allocation-*")).is_dir())
        self.assertTrue(next(self.output.glob("broker-*")).is_dir())

    def test_rejects_unsafe_parent_and_release_or_real_profile_build_before_dispatch(self):
        self.output.mkdir(mode=0o755)
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.private_parent(self.output)
        link = self.root / "linked-parent"
        link.symlink_to(self.output, target_is_directory=True)
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.private_parent(link)
        for mismatch in [
            {**stage_macos.SYNTHETIC_PROFILE_METADATA, "build_type": "Release"},
            {**stage_macos.SYNTHETIC_PROFILE_METADATA, "production_accepted": True},
        ]:
            (self.build / stage_macos.SYNTHETIC_PROFILE_BUILD).write_text(json.dumps(mismatch))
            with self.boundaries(), self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.prepare(self.options())
        self.assertEqual(self.commands, [])
        (self.build / stage_macos.SYNTHETIC_PROFILE_BUILD).write_text(
            json.dumps(stage_macos.SYNTHETIC_PROFILE_METADATA))
        (self.build / stage_macos.PROFILE_CRYPTO_BUILD).write_text(
            json.dumps(stage_macos.PROFILE_CRYPTO_METADATA))
        with self.boundaries(), self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.prepare(self.options())
        self.assertEqual(self.commands, [])

    def test_policy_rejects_placeholder_ids_and_control_character_paths(self):
        for session, port in [(0, 123), (2**32 - 1, 123), (123, 0), (123, 65536)]:
            with self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.policy_bytes(session, port, self.root, self.root / "Host.app",
                                        self.root / "broker", self.personal)
        for path in [Path("relative"), Path("/private/tmp/bad\npath"), Path("/private/tmp/é")]:
            with self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.policy_path(path)

    def test_toolchain_gate_checks_both_compiler_and_cargo(self):
        with patch.object(diagnostic, "run_logged", side_effect=[b"rustc 1.96.0 (fixture)\n",
                                                                  b"cargo 1.96.0 (fixture)\n"]) as run:
            diagnostic.require_toolchain({}, self.root)
        self.assertEqual([call.args[0] for call in run.call_args_list],
                         [["rustc", "--version"], ["cargo", "--version"]])
        with patch.object(diagnostic, "run_logged", side_effect=[b"rustc 1.96.0 (fixture)\n",
                                                                  b"cargo 1.97.1 (fixture)\n"]):
            with self.assertRaises(diagnostic.DiagnosticError):
                diagnostic.require_toolchain({}, self.root)

    def test_fixture_override_cannot_change_the_core_fixture_run(self):
        with self.boundaries(), self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.prepare(self.options("--fixture-origin", "http://127.0.0.1:60924"))
        self.assertEqual(self.commands, [])

    def test_stage_only_compiler_seal_binds_the_exact_selected_fixture(self):
        with self.boundaries():
            evidence, _ = diagnostic.prepare(self.options(
                "--stage-only", "--fixture-origin", "http://127.0.0.1:60924"))
        seal = json.loads((evidence / "desktop-compiler-seal.json").read_text())
        self.assertEqual(seal["fixture_origin"], "http://127.0.0.1:60924")
        self.assertEqual(len(self.commands), 1)

    def test_zero_test_success_cannot_become_native_acceptance(self):
        evidence = self.root / "test-evidence"
        evidence.mkdir()
        log = evidence / "core-test.log"
        screenshot = evidence / "mac-core.png"
        log.write_bytes(b"running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored;\n")
        screenshot.write_bytes(b"\x89PNG\r\n\x1a\nfixture")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.verify_core_result(evidence)
        log.write_text(f"running 1 test\ntest {diagnostic.CORE_TEST} ... ok\n"
                       "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out;\n")
        diagnostic.verify_core_result(evidence)
        screenshot.write_bytes(b"invalid picture")
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.verify_core_result(evidence)

    def test_transfer_result_requires_its_exact_case_and_completion_receipt(self):
        evidence = self.root / "transfer-evidence"
        evidence.mkdir()
        log = evidence / "core-test.log"
        result = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n"
        log.write_text(f"test {diagnostic.TRANSFER_TEST} ... ok\n" + result)
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.verify_core_result(evidence, "transfers")
        receipt = (diagnostic.TRANSFER_RECEIPT +
                   b" direct-child shutdown passed; production acceptance false\n").decode()
        log.write_text(receipt + f"test {diagnostic.CORE_TEST} ... ok\n" + result)
        with self.assertRaises(diagnostic.DiagnosticError):
            diagnostic.verify_core_result(evidence, "transfers")
        log.write_text(receipt + f"test {diagnostic.TRANSFER_TEST} ... ok\n" + result)
        diagnostic.verify_core_result(evidence, "transfers")


if __name__ == "__main__":
    unittest.main()
