"""Real-component proof contracts; no native process, browser or Security runs."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import prepare_owned_custody_proof as owned


def receipt(role):
    return {"schema_version": 1, "role": role, "pre_body_denials": 511,
            "direct_child_reaped": True, "exit_code": 0, "cef_loaded": False,
            "production_accepted": False}


class ProofContracts(unittest.TestCase):
    def test_fixed_roles_never_substitute_plugin_or_alerts_for_generic_front(self):
        paths = owned.role_paths(Path("/private/allocation/Browser.app"))
        self.assertEqual(len(paths), 5)
        self.assertEqual(paths[3], paths[4])
        self.assertIn(" (Renderer)", str(paths[1]))
        self.assertIn(" (GPU)", str(paths[2]))
        self.assertTrue(all(" (Plugin)" not in str(p) and " (Alerts)" not in str(p) for p in paths))

    @unittest.skipUnless(os.name == "posix", "POSIX ownership/private-mode custody roots run on Linux/macOS CI")
    def test_policy_binds_original_app_and_separate_canonical_custody_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary).resolve()
            allocation = parent / "allocation"
            allocation.mkdir(mode=0o700)
            root = parent / "custody"
            root.mkdir(mode=0o700)
            app = allocation / "Browser.app"
            raw = (f"COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n42\n19431\n{allocation}\n{app}\n"
                   f"com.obscuritylabs.fixture\n{allocation / 'profile'}\n{root}\n/Users/fixture\n").encode()
            self.assertEqual(owned.policy_namespace(raw, allocation, app), root)
            for value in (raw.rstrip(), raw + b"extra\n", raw.replace(b"\n42\n", b"\n042\n"),
                          raw.replace(str(root).encode(), str(allocation).encode()),
                          raw.replace(str(root).encode(), (str(root) + "/").encode()),
                          raw.replace(str(root).encode(), (str(parent) + "/../custody").encode())):
                with self.subTest(value=value), self.assertRaises((owned.common.DiagnosticError, OSError)):
                    owned.policy_namespace(value, allocation, app)

    @unittest.skipUnless(os.name == "posix", "POSIX ownership/private-mode and no-follow evidence runs on Linux/macOS CI")
    def test_private_result_inputs_reject_links_public_permissions_and_size(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = root / "result.json"
            owned.common.write_json(value, {"exit_code": 0})
            self.assertEqual(owned.private_json(value, 128), {"exit_code": 0})
            value.chmod(0o644)
            with self.assertRaises(owned.common.DiagnosticError):
                owned.private_json(value, 128)
            value.chmod(0o600)
            alias = root / "alias"
            os.link(value, alias)
            with self.assertRaises(owned.common.DiagnosticError):
                owned.private_json(value, 128)
            alias.unlink()
            alias.symlink_to(value)
            with self.assertRaises(OSError):
                owned.private_json(alias, 128)
            with self.assertRaises(owned.common.DiagnosticError):
                owned.private_json(value, 1)

    def test_every_positive_native_denial_and_reap_is_mandatory(self):
        prepared = {"component_manifest_sha256": "a" * 64, "policy_sha256": "b" * 64,
                    "native_source_sha256": {k: "c" * 64 for k in owned.SOURCE_KEYS},
                    "roles": [{"role": i, "front_cdhash": str(i + 1) * 40} for i in range(5)]}
        rows = [(item, receipt(i), b"", {"exit_code": 0}) for i, item in enumerate(prepared["roles"])]
        proof = owned.validate_results(prepared, rows)
        self.assertEqual(proof["purpose"], "colossus-macos-owned-process-custody-denials")
        self.assertEqual(set(proof["native_source_sha256"]), owned.SOURCE_KEYS)
        self.assertFalse(proof["production_accepted"])
        self.assertEqual([r["role"] for r in proof["roles"]], list(range(5)))
        for changes in ({"pre_body_denials": 255}, {"direct_child_reaped": False},
                        {"cef_loaded": True}, {"production_accepted": True}, {"exit_code": 1},
                        {"role": 3}, {"schema_version": True}, {"extra": True}):
            changed = copy.deepcopy(rows)
            changed[2][1].update(changes)
            with self.subTest(changes=changes), self.assertRaises(owned.common.DiagnosticError):
                owned.validate_results(prepared, changed)
        for changed in (rows[:4], [*rows[:4], (rows[4][0], rows[4][1], b"fatal", rows[4][3])],
                        [*rows[:4], (rows[4][0], rows[4][1], b"", {"exit_code": False})]):
            with self.assertRaises(owned.common.DiagnosticError):
                owned.validate_results(prepared, changed)

    @unittest.skipUnless(os.name == "posix", "POSIX ownership/private-mode and no-follow markers run on Linux/macOS CI")
    def test_marker_tamper_or_unexpected_destination_preserves_unknown_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            root.chmod(0o700)
            profile = root / "profile"
            profile.mkdir()
            for name in owned.MARKERS:
                owned.common.write_private(root / name, owned.CONTENT)
            original = owned.marker_state(root, profile)
            path = root / owned.MARKERS[0]
            path.write_bytes(b"changed")
            with self.assertRaises(owned.common.DiagnosticError):
                owned.marker_state(root, profile)
            self.assertTrue(path.exists())
            path.write_bytes(owned.CONTENT)
            # Make the metadata generation change explicit on coarse timestamp filesystems.
            restored = path.stat()
            os.utime(path, ns=(restored.st_atime_ns, restored.st_mtime_ns + 1_000_000_000))
            self.assertNotEqual(owned.marker_state(root, profile), original)
            (profile / ".colossus-custody-probe-alias").symlink_to(path)
            with self.assertRaises(owned.common.DiagnosticError):
                owned.marker_state(root, profile)

    def fixture(self, parent):
        root = parent / "custody"
        root.mkdir(mode=0o700)
        allocation = parent / "allocation"
        allocation.mkdir(mode=0o700)
        app = allocation / "Real Browser.app"
        paths = owned.role_paths(app)
        for path in set(paths):
            path.parent.mkdir(parents=True, exist_ok=True)
            owned.common.write_private(path, b"actual fixed front")
            owned.common.write_private(path.with_name(path.name + " Body.dylib"), b"actual CEF body")
        stage = {"allocation_root": str(allocation), "app_bundle": str(app), "broker_root": str(root),
                 "descriptor_sha256": "a" * 64}
        descriptor = {"manifest_sha256": "b" * 64, "policy_sha256": "c" * 64,
                      "front_cdhash": "d" * 40, "helper_cdhashes": ["e" * 40] * 4}
        sources = {name: "f" * 64 for name in owned.SOURCE_KEYS}
        manifest = {"owned_process": {"native_source_sha256": sources}}
        stage_path = parent / "stage.json"
        owned.common.write_json(stage_path, stage)
        return stage_path, (stage, descriptor, manifest, root)

    @unittest.skipUnless(os.name == "posix", "POSIX private-file preparation runs on Linux/macOS CI")
    def test_preparation_only_compiles_and_signs_controller_without_replacing_real_images(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary).resolve()
            stage_path, values = self.fixture(parent)
            app = Path(values[0]["app_bundle"])
            original = {p: p.read_bytes() for p in app.rglob("*") if p.is_file()}
            calls = []

            def run(command, *_args, **_kwargs):
                calls.append(command)
                if command[0] == "/usr/bin/clang":
                    owned.common.write_private(Path(command[-1]), b"controller executable")

            with patch.object(owned, "inspect_stage", return_value=values), \
                    patch.object(owned.common, "run_logged", side_effect=run), \
                    patch.object(owned.common, "cdhash", return_value="1" * 40):
                prepared_path = owned.prepare(stage_path, parent / "evidence-parent")
            prepared = owned.private_json(prepared_path, 32768)
            self.assertFalse(prepared["native_acceptance"])
            self.assertEqual({p: p.read_bytes() for p in original}, original)
            self.assertTrue(all(command[0] in ("/usr/bin/clang", "/usr/bin/codesign") for command in calls))
            self.assertTrue(all(str(app) not in argument for command in calls for argument in command))
            self.assertEqual([r["role"] for r in prepared["roles"]], list(range(5)))
            self.assertTrue(all(len(r["command"]) == 7 for r in prepared["roles"]))
            self.assertEqual(owned.marker_state(values[3], Path(values[0]["allocation_root"]) / "profile"),
                             prepared["marker_generations"])

    @unittest.skipUnless(os.name == "posix", "POSIX private evidence/marker collection runs on Linux/macOS CI")
    def test_collect_removes_only_owned_markers_after_all_actual_results_pass(self):
        for denied in (False, True):
            with self.subTest(denied=denied), tempfile.TemporaryDirectory() as temporary:
                parent = Path(temporary).resolve()
                stage_path, values = self.fixture(parent)

                def run(command, *_args, **_kwargs):
                    if command[0] == "/usr/bin/clang":
                        owned.common.write_private(Path(command[-1]), b"controller executable")

                with patch.object(owned, "inspect_stage", return_value=values), \
                        patch.object(owned.common, "run_logged", side_effect=run), \
                        patch.object(owned.common, "cdhash", return_value="1" * 40):
                    prepared_path = owned.prepare(stage_path, parent / "evidence-parent")
                    evidence = prepared_path.parent
                    unknown = values[3] / "unrelated-owned-identity"
                    owned.common.write_private(unknown, b"keep private custody")
                    for role in range(5):
                        result = receipt(role)
                        if denied and role == 3:
                            result["pre_body_denials"] = 255
                        owned.common.write_json(evidence / f"role-{role}.stdout.json", result)
                        owned.common.write_private(evidence / f"role-{role}.stderr.txt", b"")
                        owned.common.write_json(evidence / f"role-{role}.exit.json", {"exit_code": 0})
                    if denied:
                        with self.assertRaises(owned.common.DiagnosticError):
                            owned.collect(prepared_path)
                        self.assertFalse((evidence / owned.DENIALS_NAME).exists())
                        self.assertTrue(all((values[3] / name).exists() for name in owned.MARKERS))
                    else:
                        target = owned.collect(prepared_path)
                        proof = owned.private_json(target, 32768)
                        self.assertEqual(len(proof["roles"]), 5)
                        self.assertTrue(all(not (values[3] / name).exists() for name in owned.MARKERS))
                    self.assertEqual(unknown.read_bytes(), b"keep private custody")


if __name__ == "__main__":
    unittest.main()
