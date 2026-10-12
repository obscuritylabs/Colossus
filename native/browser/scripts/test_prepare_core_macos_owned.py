"""Pure preparation contracts: no Core/CEF/process/Keychain launch or signing."""
import importlib.util
import json
import os
from pathlib import Path
import stat
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

import prepare_core_macos_owned as owned


class PreparationContracts(unittest.TestCase):
    def test_preparation_seals_native_parent_after_core_compile_and_never_launches_core(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cef, native = root / "cef", root / "native"
            cef.mkdir()
            native.mkdir()
            host = root / "libcolossus_native_browser_host.dylib"
            host.write_bytes(b"fake compiled body")
            host.chmod(0o755)
            for name, value in ((owned.stage_macos.SYNTHETIC_PROFILE_BUILD,
                                 owned.stage_macos.OWNED_PROFILE_METADATA),
                                (owned.stage_macos.NETWORK_ENVELOPE_BUILD,
                                 owned.stage_macos.NETWORK_ENVELOPE_METADATA)):
                (native / name).write_text(json.dumps(value))
            calls = []
            core = root / "compiled-core"

            def run(arguments, environment, log, **_):
                calls.append((arguments[:], environment.copy()))
                if "--example" in arguments:
                    mode, allocation, seed = arguments[-3:]
                    if mode == "trust":
                        owned.common.write_private(Path(seed), b"s" * 32)
                        owned.common.write_json(Path(allocation) / "compiler-trust.json", {
                            "schema_version": 1, "purpose": owned.PURPOSE,
                            "publisher": "colossus-macos-development", "package_root": allocation,
                            "key_id": "a" * 64, "public_key": "public-key-only"})
                    return b""
                if arguments[:2] == ["cargo", "test"]:
                    core.write_bytes(b"fake executable")
                    core.chmod(0o755)
                    return json.dumps({"reason": "compiler-artifact", "target": {
                        "name": "colossus_runtime", "kind": ["lib"]},
                        "profile": {"test": True}, "executable": str(core)}).encode()
                if "stage_host_macos.py" in " ".join(arguments):
                    app = Path(arguments[arguments.index("--app") + 1])
                    app.mkdir()
                    resources = app / "Contents/Resources"
                    resources.mkdir(parents=True)
                    (resources / owned.stage_macos.NETWORK_ENVELOPE_RESOURCE).write_bytes(b"policy")
                    app.with_name(app.name + ".browser-component.json").write_bytes(b"inventory")
                return b""

            options = SimpleNamespace(cef_root=cef, native_build=native, host_binary=host,
                                      output_root=root / "evidence-parent", cargo_target_dir=None,
                                      fixture_origin=None)
            with patch.object(owned.common, "macos_platform", return_value="macosarm64"), \
                    patch.object(owned.owned_process_component, "verify_build"), \
                    patch.object(owned.common, "require_toolchain"), \
                    patch.object(owned.common, "current_audit_session", return_value=1234), \
                    patch.object(owned.common, "personal_home", return_value=root), \
                    patch.object(owned.common, "cdhash", return_value="b" * 40), \
                    patch.object(owned, "fixture_certificates", return_value=b"public-ca"), \
                    patch.object(owned, "compile_core", side_effect=lambda environment, log:
                                 run(owned.core_build_command(), environment, log)), \
                    patch.object(owned.common, "run_logged", side_effect=run):
                evidence = owned.prepare(options)
            receipt = json.loads((evidence / "owned-core-stage.json").read_text())
            self.assertFalse(receipt["native_acceptance"])
            self.assertTrue(receipt["signing_seed_removed"])
            self.assertFalse(any(Path(receipt["broker_root"]).rglob("seed")))
            self.assertFalse(any(arguments[0] == str(core) for arguments, _ in calls))
            core_index = next(i for i, (arguments, _) in enumerate(calls)
                              if arguments[:2] == ["cargo", "test"])
            self.assertIn("--no-run", calls[core_index][0])
            trust = json.loads(calls[core_index][1][owned.TRUST_ENV])
            self.assertEqual(len(trust), 6)
            self.assertNotIn("seed", calls[core_index][1])
            native_calls = [(i, arguments, environment) for i, (arguments, environment)
                            in enumerate(calls) if arguments[:2] == ["cargo", "build"]]
            self.assertEqual(len(native_calls), 2)
            for index, _, environment in native_calls:
                self.assertGreater(index, core_index)
                self.assertEqual(environment["COLOSSUS_MAC_PROCESS_PARENT_CDHASH"], "b" * 40)
                self.assertEqual(environment["COLOSSUS_TLS_BROKER_PARENT_CDHASH"], "b" * 40)
                diagnostic = Path(environment["COLOSSUS_MAC_PROCESS_DIAGNOSTIC_ROOT"])
                self.assertEqual(diagnostic, Path(receipt["broker_root"]) / "process-diagnostics")
                self.assertEqual(stat.S_IMODE(diagnostic.stat().st_mode), 0o700)
                self.assertFalse(any(diagnostic.iterdir()))
            process_command = next(arguments for _, arguments, _ in native_calls if "process-broker/Cargo.toml" in " ".join(arguments))
            self.assertIn("macos-process-output-diagnostic", process_command[process_command.index("--features") + 1])
            self.assertEqual(receipt["command"][1], owned.CORE_TEST)

    def test_failed_compiler_json_diagnostics_are_preserved_without_a_native_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "compile.log"
            diagnostics = b'{"reason":"compiler-message","message":{"level":"error"}}\n'
            with patch.object(owned.subprocess, "run", return_value=owned.subprocess.CompletedProcess(
                    owned.core_build_command(), 101, diagnostics)) as run:
                with self.assertRaises(owned.subprocess.CalledProcessError):
                    owned.compile_core({}, log)
            self.assertEqual(log.read_bytes(), diagnostics)
            self.assertIn("--no-run", run.call_args.args[0])

    def test_only_exact_numeric_https_fixture_origins_are_accepted(self):
        self.assertEqual(owned.fixture_port("https://127.0.0.1:19431"), 19431)
        for value in ("http://127.0.0.1:19431", "https://localhost:19431",
                      "https://127.0.0.1", "https://127.0.0.1:443",
                      "https://127.0.0.1:019431", "https://127.0.0.1:19431/",
                      "https://user@127.0.0.1:19431", "https://[::1]:19431",
                      "https://127.0.0.1:19431?token=x", "https://127.0.0.1:65536"):
            with self.subTest(value=value), self.assertRaises(owned.common.DiagnosticError):
                owned.fixture_port(value)

    def test_compiler_identifies_exact_runtime_lib_test_not_another_executable(self):
        artifact = {"reason": "compiler-artifact", "target": {
            "name": "colossus_runtime", "kind": ["lib"]},
            "profile": {"test": True}, "executable": "/private/tmp/core-test"}
        self.assertEqual(owned.core_executable(json.dumps(artifact).encode()),
                         Path("/private/tmp/core-test"))
        self.assertEqual(owned.core_executable(
            b"compiler diagnostic\n" + json.dumps(artifact).encode()), Path("/private/tmp/core-test"))
        for changes in ({"reason": "build-script-executed"},
                        {"profile": {"test": False}}, {"profile": {"test": 1}},
                        {"target": {"name": "colossus_sidecar", "kind": ["bin"]}},
                        {"executable": "relative-core-test"}):
            with self.subTest(changes=changes), self.assertRaises(owned.common.DiagnosticError):
                owned.core_executable(json.dumps({**artifact, **changes}).encode())
        with self.assertRaises(owned.common.DiagnosticError):
            owned.core_executable(b"{}\n")
        with self.assertRaises(owned.common.DiagnosticError):
            owned.core_executable((json.dumps(artifact) + "\n" + json.dumps({
                **artifact, "executable": "/private/tmp/other-core-test"})).encode())

    def test_build_commands_cannot_launch_or_filter_other_native_tests(self):
        command = owned.core_build_command()
        self.assertIn("--no-run", command)
        self.assertIn("--lib", command)
        self.assertEqual(command[-1], owned.CORE_TEST)
        self.assertNotIn("--ignored", command)
        for kind in ("process", "tls"):
            command = owned.native_build_command(kind)
            self.assertEqual(command[:2], ["cargo", "build"])
            self.assertNotIn("test", command)
            self.assertNotIn("run", command)
        with self.assertRaises(owned.common.DiagnosticError):
            owned.native_build_command("arbitrary-entry")

    def test_stage_command_requires_both_fixed_native_brokers_and_owned_lane(self):
        path = Path("/private/tmp/fixture")
        command = owned.stage_command(path, path, path, path, "macosarm64", path, path, path)
        for flag in ("--owned-process-development", "--process-broker", "--tls-broker",
                     "--macos-synthetic-profile-diagnostic", "--macos-network-envelope-policy"):
            self.assertIn(flag, command)
        self.assertNotIn("--macos-owned-profile-development", command)
        self.assertNotIn("--no-sandbox", command)

    def test_helper_seals_match_native_bootstrap_role_image_binding(self):
        # Renderer/GPU are typed bundles; network/utility use the generic image
        # and differ only by the authenticated native CBOOT role.
        self.assertEqual(owned.HELPER_SUFFIXES, (" (Renderer)", " (GPU)", "", ""))

    def test_frozen_core_survives_later_compiler_output_replacement(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compiled = root / "compiled-core"
            owned.common.write_private(compiled, b"immutable first Core binary")
            compiled.chmod(0o700)
            frozen = owned.freeze_core(compiled, root)
            compiled.write_bytes(b"later Cargo rebuild")
            self.assertEqual(frozen.read_bytes(), b"immutable first Core binary")
            self.assertNotEqual(frozen.stat().st_ino, compiled.stat().st_ino)
            self.assertEqual(frozen.stat().st_nlink, 1)
            self.assertEqual(stat.S_IMODE(frozen.stat().st_mode), 0o700)

    def test_signed_descriptor_only_pins_one_exact_loopback_tls_destination(self):
        facts = {key: "a" * (64 if key.endswith("sha256") else 40) for key in (
            "manifest_sha256", "front_cdhash", "body_cdhash", "policy_sha256",
            "broker_cdhash", "tls_cdhash")}
        value = owned.descriptor(Path("/private/tmp/allocation"),
                                 Path("/private/tmp/allocation/App.app"),
                                 "https://127.0.0.1:19431", b"reviewed-public-root", facts,
                                 ["b" * 40] * 4)
        self.assertEqual(value["tls"]["destinations"], ["127.0.0.1:19431"])
        self.assertEqual(value["tls"]["public_roots"], [list(b"reviewed-public-root")])
        self.assertFalse(value["production_accepted"])
        self.assertIsNone(value["custody_denials_sha256"])
        for root in (b"", b"x" * 8193):
            with self.assertRaises(owned.common.DiagnosticError):
                owned.descriptor(Path("/a"), Path("/a/App.app"),
                                 "https://127.0.0.1:19431", root, facts, ["b"] * 4)

    def test_only_exact_preparer_seed_inode_can_be_removed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            seed = root / "seed"
            owned.common.write_private(seed, b"a" * 32)
            identity = owned.seed_identity(seed)
            seed.rename(root / "original")
            owned.common.write_private(seed, b"b" * 32)
            with self.assertRaises(owned.common.DiagnosticError):
                owned.remove_owned_seed(seed, identity)
            self.assertEqual(seed.read_bytes(), b"b" * 32)
            self.assertEqual((root / "original").read_bytes(), b"a" * 32)
            owned.remove_owned_seed(seed, owned.seed_identity(seed))
            self.assertFalse(seed.exists())

    def test_seed_alias_and_public_permissions_are_retained(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            seed = root / "seed"
            owned.common.write_private(seed, b"a" * 32)
            seed.chmod(0o644)
            with self.assertRaises(owned.common.DiagnosticError):
                owned.seed_identity(seed)
            seed.chmod(0o600)
            os.link(seed, root / "alias")
            with self.assertRaises(owned.common.DiagnosticError):
                owned.seed_identity(seed)
            self.assertTrue(seed.exists())

    @unittest.skipUnless(importlib.util.find_spec("cryptography"), "cryptography prerequisite absent")
    def test_synthetic_fixture_has_exact_ip_san_private_pkcs8_and_normal_chain(self):
        from cryptography import x509
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import ec

        with tempfile.TemporaryDirectory() as temporary:
            inputs = Path(temporary)
            root_bytes = owned.fixture_certificates(inputs)
            ca = x509.load_der_x509_certificate(root_bytes)
            leaf = x509.load_der_x509_certificate((inputs / "fixture-leaf.der").read_bytes())
            key = serialization.load_der_private_key((inputs / "fixture-key.pk8").read_bytes(), None)
            self.assertEqual(leaf.extensions.get_extension_for_class(
                x509.SubjectAlternativeName).value.get_values_for_type(x509.IPAddress),
                [owned.ipaddress.ip_address("127.0.0.1")])
            self.assertEqual(key.public_key().public_numbers(), leaf.public_key().public_numbers())
            ca.public_key().verify(leaf.signature, leaf.tbs_certificate_bytes, ec.ECDSA(hashes.SHA256()))
            self.assertEqual(leaf.issuer, ca.subject)
            for name in ("fixture-ca.der", "fixture-leaf.der", "fixture-key.pk8"):
                metadata = (inputs / name).stat()
                self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o600)
                self.assertEqual(metadata.st_nlink, 1)
            with self.assertRaises(FileExistsError):
                owned.fixture_certificates(inputs)


if __name__ == "__main__":
    unittest.main()
