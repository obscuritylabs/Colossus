"""Offline cryptographic fixture contracts; no native calls or launches."""
from __future__ import annotations
import json
import os
from pathlib import Path
import tempfile
import unittest
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.serialization import pkcs12
import owned_pki_inputs as inputs


class OwnedInputs(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="colossus-pki-input-source-")
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        self.broker, self.fixture, self.workspace = [self.root / name for name in
            ("broker", "fixture", "workspace")]
        for path in (self.broker, self.fixture, self.workspace):
            path.mkdir(mode=0o700)

    def tearDown(self):
        self.temporary.cleanup()

    def test_exact_catalog_scope_hashes_and_native_compatible_container(self):
        facts = inputs.prepare(self.broker, self.fixture, self.workspace, "https://127.0.0.1:4433")
        registry_bytes = (self.broker / "owned-pki-registry.json").read_bytes()
        metadata = json.loads(registry_bytes)
        self.assertEqual(facts["registry_sha256"], inputs.sha256(registry_bytes))
        review = metadata["reviews"][0]
        self.assertEqual(review["workspace_id"], inputs.workspace_id(self.workspace))
        self.assertEqual(review["application_id"], "authenticated-app")
        self.assertEqual(review["scope"], {"kind": "conversation", "id": "conversation"})
        directory = self.broker / "pki-enrollment"
        password = (directory / "password-0").read_bytes()
        key, client, chain = pkcs12.load_key_and_certificates(
            (directory / "client-0.p12").read_bytes(), password)
        leaf = (directory / "client-0.der").read_bytes()
        self.assertEqual(client.public_bytes(serialization.Encoding.DER), leaf)
        self.assertEqual(key.public_key().public_numbers(), client.public_key().public_numbers())
        self.assertEqual(chain[0].public_bytes(serialization.Encoding.DER), facts["public_ca_der"])
        self.assertEqual(client.signature_hash_algorithm.name, "sha256")
        self.assertEqual(chain[0].signature_hash_algorithm.name, "sha256")
        self.assertEqual(client.fingerprint(hashes.SHA256()).hex(), facts["reviewed_leaf_sha256"])
        identity = metadata["identities"][0]
        for name, field in (("client-0.der", "leaf_sha256"), ("client-0.p12", "pkcs12_sha256"),
                            ("password-0", "password_sha256")):
            self.assertEqual(inputs.sha256((directory / name).read_bytes()), identity[field])
        server = x509.load_der_x509_certificate((self.fixture / "fixture-leaf.der").read_bytes())
        self.assertEqual(server.signature_hash_algorithm.name, "sha256")
        self.assertEqual(set(os.listdir(self.fixture)), {"fixture-ca.der", "fixture-client.der",
            "fixture-leaf.der", "fixture-key.pk8"})
        for path in directory.iterdir():
            self.assertEqual(path.stat().st_mode & 0o7777, 0o600)
            self.assertEqual(path.stat().st_nlink, 1)
        self.assertFalse(facts["native_acceptance"])

    def test_prior_native_duty_symlink_and_foreign_workspace_remain_untouched(self):
        orphan = self.broker / "colossus-custody-generation.jsonl"
        orphan.write_bytes(b"retained-duty")
        with self.assertRaises(ValueError):
            inputs.prepare(self.broker, self.fixture, self.workspace, "https://127.0.0.1:4433")
        self.assertEqual(orphan.read_bytes(), b"retained-duty")
        self.assertFalse((self.broker / "pki-enrollment").exists())
        orphan.unlink()  # This is a plain synthetic test marker, never native state.
        alias = self.root / "workspace-alias"
        alias.symlink_to(self.workspace, target_is_directory=True)
        with self.assertRaises(ValueError):
            inputs.prepare(self.broker, self.fixture, alias, "https://127.0.0.1:4433")
        (self.workspace / "existing-state").write_bytes(b"retained")
        with self.assertRaises(ValueError):
            inputs.prepare(self.broker, self.fixture, self.workspace, "https://127.0.0.1:4433")
        self.assertEqual((self.workspace / "existing-state").read_bytes(), b"retained")

    def test_invalid_origins_do_not_write_any_inputs(self):
        for origin in ("http://127.0.0.1:4433", "https://127.0.0.1:443", "https://localhost:4433",
                       "https://127.0.0.1:4433/", "https://user@127.0.0.1:4433"):
            with self.assertRaises(ValueError):
                inputs.prepare(self.broker, self.fixture, self.workspace, origin)
        self.assertEqual(os.listdir(self.broker), [])
        self.assertEqual(os.listdir(self.fixture), [])


if __name__ == "__main__":
    unittest.main()
