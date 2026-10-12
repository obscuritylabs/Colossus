#!/usr/bin/env python3
"""Pure preparer/receipt tests: no native fixture, browser or audit API runs."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_native_acceptance as runner


class PreparerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="tls-preparer-pure-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.calls = []

    def tool(self, argv, environment, log, timeout=1800):
        self.calls.append((argv, environment.copy()))
        output = b""
        if argv[:2] in (["rustc", "--version"], ["cargo", "--version"]):
            output = (argv[0] + " 1.96.0 (pure fixture)\n").encode()
        elif argv[:2] == ["cargo", "build"]:
            target = Path(argv[argv.index("--target-dir") + 1])
            binary = target / ("debug/examples/native-parent-acceptance" if "--example" in argv else "debug/colossus-native-browser-tls-broker")
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"pure-image:" + environment.get("COLOSSUS_TLS_BROKER_PARENT_CDHASH", "parent").encode())
        elif argv[:2] == ["/usr/bin/codesign", "--force"]:
            path = Path(argv[-1])
            path.write_bytes(path.read_bytes() + b"|pure-signature")
        elif argv[:2] == ["/usr/bin/codesign", "-d"]:
            output = ("CDHash=" + hashlib.sha256(Path(argv[-1]).read_bytes()).hexdigest()[:40] + "\n").encode()
        elif argv[:2] != ["/usr/bin/codesign", "--verify"]:
            self.fail("preparation attempted a native launch or unsupported tool")
        runner.write_private(log, output)
        return output

    def prepare(self):
        output = self.directory / "evidence"
        with patch.object(runner, "require_platform"), patch.object(runner, "run_tool", side_effect=self.tool), patch.object(runner, "source_digests", return_value={"fixture": "a" * 64}):
            return runner.prepare(output)

    def test_default_preparation_never_selects_native_launch(self):
        args = runner.parser().parse_args(["--output-root", str(self.directory)])
        self.assertFalse(args.run)
        receipt = self.prepare()
        value = runner.load_prepared(receipt)
        self.assertFalse(value["native_cases_executed"])
        self.assertFalse(value["production_enabled"])
        self.assertEqual(set(value["commands"]), set(runner.CASES))
        self.assertTrue(all(argv[0] in ("rustc", "cargo", "/usr/bin/codesign") for argv, _ in self.calls))

    def test_parent_is_signed_before_either_child_seal_is_selected(self):
        value = json.loads(self.prepare().read_text())
        builds = [(argv, env) for argv, env in self.calls if argv[:2] == ["cargo", "build"]]
        self.assertNotIn("COLOSSUS_TLS_BROKER_PARENT_CDHASH", builds[0][1])
        self.assertEqual(builds[1][1]["COLOSSUS_TLS_BROKER_PARENT_CDHASH"], value["parent"]["cdhash"])
        self.assertEqual(builds[2][1]["COLOSSUS_TLS_BROKER_PARENT_CDHASH"], value["wrong_parent_cdhash"])
        self.assertNotEqual(value["wrong_parent_cdhash"], value["parent"]["cdhash"])
        self.assertNotEqual(builds[0][0][builds[0][0].index("--target-dir") + 1], builds[1][0][builds[1][0].index("--target-dir") + 1])

    def test_commands_carry_only_public_operator_metadata_and_correct_wrong_image(self):
        value = json.loads(self.prepare().read_text())
        wrong = value["commands"]["wrong-parent"]
        self.assertEqual(wrong[wrong.index("--broker") + 1], value["wrong_broker"]["path"])
        self.assertEqual(wrong[wrong.index("--broker-cdhash") + 1], value["wrong_broker"]["cdhash"])
        self.assertEqual(wrong[wrong.index("--parent-cdhash") + 1], value["parent"]["cdhash"])
        self.assertNotIn("authentication", json.dumps(value))
        self.assertNotIn("credential", json.dumps(value))
        self.assertNotIn("generation", json.dumps(value))

    def test_changed_image_is_rejected_before_a_native_command(self):
        receipt = self.prepare()
        value = json.loads(receipt.read_text())
        image = Path(value["broker"]["path"])
        image.chmod(0o700)
        image.write_bytes(b"changed")
        image.chmod(0o500)
        with self.assertRaises(runner.AcceptanceError):
            runner.load_prepared(receipt)

    def test_changed_command_and_nonprivate_receipt_are_rejected(self):
        receipt = self.prepare()
        value = json.loads(receipt.read_text())
        value["commands"]["positive"].append("--unsafe")
        receipt.write_text(json.dumps(value))
        with self.assertRaises(runner.AcceptanceError):
            runner.load_prepared(receipt)
        receipt.chmod(0o644)
        with self.assertRaises(runner.AcceptanceError):
            runner.load_prepared(receipt)

    def test_source_mutation_during_preparation_preserves_and_rejects(self):
        output = self.directory / "evidence"
        with patch.object(runner, "require_platform"), patch.object(runner, "run_tool", side_effect=self.tool), patch.object(runner, "source_digests", side_effect=[{"fixture": "a" * 64}, {"fixture": "b" * 64}]):
            with self.assertRaises(runner.AcceptanceError):
                runner.prepare(output)
        allocations = list(output.iterdir())
        self.assertEqual(len(allocations), 1)
        self.assertTrue((allocations[0] / "native-parent-acceptance").is_file())
        self.assertFalse((allocations[0] / "prepared.json").exists())


class EvidenceTests(unittest.TestCase):
    def result(self):
        return dict(accepted=True, broker_exit_observed=True, listener_v4_closed=True, listener_v6_closed=True,
                    stdout_empty=True, forced=False, full_helper_cleanup_proven=False, production_enabled=False,
                    normal_tls=True, leaf_pin_matches=True, upstream_observed=True, proxy_credential_stripped=True,
                    tracked_socket_closed=True, broker_reaped=True, broker_exit_code=0, shutdown_ack=True)

    def test_positive_requires_real_tls_leaf_pin_and_clean_acknowledged_exit(self):
        result = self.result()
        runner.verify_result("positive", result)
        for field in ("normal_tls", "leaf_pin_matches", "upstream_observed", "shutdown_ack", "broker_reaped"):
            altered = result.copy()
            altered[field] = False
            with self.assertRaises(runner.AcceptanceError):
                runner.verify_result("positive", altered)

    def test_crash_exit_event_cannot_be_misrepresented_as_reaping_or_graceful_shutdown(self):
        result = self.result()
        result.update(broker_reaped=False, broker_exit_code=24, shutdown_ack=False, parent_reaped=True,
                      parent_crashed_without_drop=True, parent_exit_code=73)
        runner.verify_result("parent-crash", result)
        for field, value in (("broker_reaped", True), ("shutdown_ack", True), ("parent_reaped", False), ("broker_exit_code", 0), ("parent_exit_code", 0)):
            altered = result.copy()
            altered[field] = value
            with self.assertRaises(runner.AcceptanceError):
                runner.verify_result("parent-crash", altered)

    def test_wrong_parent_must_fail_before_tls_without_availability(self):
        result = self.result()
        result.update(broker_exit_code=11, normal_tls=False, shutdown_ack=False)
        runner.verify_result("wrong-parent", result)
        result["normal_tls"] = True
        with self.assertRaises(runner.AcceptanceError):
            runner.verify_result("wrong-parent", result)

    def test_endpoint_denial_alone_does_not_replace_exact_process_exit_evidence(self):
        result = self.result()
        result["broker_exit_observed"] = False
        with self.assertRaises(runner.AcceptanceError):
            runner.verify_result("positive", result)


if __name__ == "__main__":
    unittest.main()
