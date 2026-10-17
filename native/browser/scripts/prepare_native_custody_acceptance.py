#!/usr/bin/env python3
"""Prepare a signed no-CEF native custody/TLS fixture; never execute it.

Requires Root's complete maintained five-front native result at the same immutable
package. Cryptographic inputs are synthetic local files; no OS store is opened.
"""
from __future__ import annotations
import argparse
import base64
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, ed25519
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID
import prepare_custody_front_probe as front

PURPOSE = "colossus-macos-native-custody-acceptance"
PUBLISHER = "colossus-macos-development"

def private_write(path: Path, data: bytes) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())

def certificate(name: str, key, issuer, issuer_key, ca: bool, server: bool = False) -> x509.Certificate:
    now = datetime.datetime.now(datetime.timezone.utc)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, name)])
    builder = x509.CertificateBuilder().subject_name(subject).issuer_name(issuer.subject if issuer else subject).public_key(key.public_key()).serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(minutes=5)).not_valid_after(now + datetime.timedelta(days=2))
    builder = builder.add_extension(x509.BasicConstraints(ca=ca, path_length=0 if ca else None), critical=True)
    builder = builder.add_extension(x509.KeyUsage(digital_signature=not ca, content_commitment=False, key_encipherment=False, data_encipherment=False, key_agreement=False, key_cert_sign=ca, crl_sign=ca, encipher_only=False, decipher_only=False), critical=True)
    if not ca:
        builder = builder.add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH if server else ExtendedKeyUsageOID.CLIENT_AUTH]), critical=False)
    if server:
        builder = builder.add_extension(x509.SubjectAlternativeName([x509.DNSName("localhost")]), critical=False)
    return builder.sign(issuer_key or key, hashes.SHA256())

def prepare(root: Path, rebuild_unlaunched: bool = False) -> Path:
    root = root.resolve(strict=True)
    if root.stat().st_uid != os.geteuid() or root.stat().st_mode & 0o7777 != 0o700:
        raise ValueError("fixture root must be retained owner-private")
    prepared = json.loads((root / "prepared.json").read_text())
    result = json.loads((root / "native-result.json").read_text())
    if result.get("prepared_sha256") != front.digest(root / "prepared.json") or len(result.get("roles", [])) != 5 or any(not role.get("accepted") for role in result["roles"]):
        raise ValueError("Root must run/review all five actual native fronts before custody preparation")
    namespace = root / "custody/colossus-tls-identity"
    journal = root / "custody/colossus-custody-generation.jsonl"
    if journal.exists() or journal.is_symlink():
        raise ValueError("prior durable native generation record must remain untouched")
    if namespace.exists() or namespace.is_symlink():
        raise ValueError("prior or uncertain real store must remain untouched")
    front.marker_generations(root)
    inputs = root / "custody/acceptance-inputs"
    if rebuild_unlaunched:
        result_path = root / "native-custody-tls-result.json"
        if result_path.exists() or result_path.is_symlink() or namespace.exists() or namespace.is_symlink() or any(path.name.startswith(".retired-tls-identity-") for path in (root / "custody").iterdir()):
            raise ValueError("only an unlaunched fixture may rebuild; native evidence/state retained")
        config = json.loads((inputs / "identity.json").read_text())
        for name, field in (("private-ca.der", "ca_sha256"), ("reviewed-client.der", "reviewed_leaf_sha256"), ("encrypted-client.p12", "pkcs12_sha256"), ("enrollment-password", "password_sha256")):
            if front.digest(inputs / name) != config[field]:
                raise ValueError("immutable synthetic input changed")
        for name in ("compiler-trust.json", "native-custody-acceptance.json", "native-custody-acceptance.bundle.json", "native-custody-launch.json", "native-custody-parent", "colossus-native-custody-acceptance"):
            path = root / name
            backup = root / (name + ".before-rebuild-" + front.digest(path))
            private_write(backup, path.read_bytes())
            backup.chmod(0o400)
            path.unlink()
    else:
        inputs.mkdir(mode=0o700)
        ca_key = ec.generate_private_key(ec.SECP256R1())
        ca = certificate("Colossus Disposable Native Custody CA", ca_key, None, None, True)
        client_key = ec.generate_private_key(ec.SECP256R1())
        client = certificate("Colossus Disposable Native Custody Client", client_key, ca, ca_key, False)
        server_key = ec.generate_private_key(ec.SECP256R1())
        server = certificate("Colossus Disposable Native Custody Server", server_key, ca, ca_key, False, True)
        password = os.urandom(32).hex().encode()
        # Native macOS legacy PKCS12 import rejects some OpenSSL3 defaults.
        # This is only disposable fixture container encoding; certificate and
        # TLS signatures remain SHA256, and the private namespace is the custody
        # boundary. See cryptography50.0.2 PKCS12 serialization compatibility.
        encryption = (serialization.PrivateFormat.PKCS12.encryption_builder()
                      .kdf_rounds(50000)
                      .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC)
                      .hmac_hash(hashes.SHA1()).build(password))
        values = {
            "private-ca.der": ca.public_bytes(serialization.Encoding.DER),
            "reviewed-client.der": client.public_bytes(serialization.Encoding.DER),
            "encrypted-client.p12": pkcs12.serialize_key_and_certificates(b"Colossus disposable client", client_key, client, [ca], encryption),
            "enrollment-password": password,
            "server.der": server.public_bytes(serialization.Encoding.DER),
            "server-key.pk8": server_key.private_bytes(serialization.Encoding.DER, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()),
        }
        for name, data in values.items():
            private_write(inputs / name, data)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        config = {"schema_version": 1, "purpose": "colossus-native-custody-synthetic-inputs", "synthetic_fixture_only": True, "origin": f"https://localhost:{port}", "identity_id": "synthetic-reviewed-client-p256",
                  "reviewed_leaf_sha256": hashlib.sha256(values["reviewed-client.der"]).hexdigest(), "ca_sha256": hashlib.sha256(values["private-ca.der"]).hexdigest(),
                  "pkcs12_sha256": hashlib.sha256(values["encrypted-client.p12"]).hexdigest(), "password_sha256": hashlib.sha256(password).hexdigest()}
        private_write(inputs / "identity.json", json.dumps(config).encode())
    signer = ed25519.Ed25519PrivateKey.generate()
    public = signer.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    key_id = hashlib.sha256(public).hexdigest()
    trust = {"schema_version": 1, "purpose": "colossus-macos-owned-process-development", "publisher": PUBLISHER, "package_root": str(root), "key_id": key_id, "public_key": base64.b64encode(public).decode()}
    private_write(root / "compiler-trust.json", json.dumps(trust).encode())
    build = root / "rust-build"
    env = os.environ.copy()
    cargo = subprocess.check_output(["rustup", "which", "--toolchain", "1.96.0", "cargo"], text=True).strip()
    env["PATH"] = str(Path(cargo).parent) + os.pathsep + env["PATH"]
    env["CARGO_TARGET_DIR"] = str(build)
    env["CARGO_BUILD_JOBS"] = "2"
    manifest = front.SOURCE / "tls-broker/Cargo.toml"
    subprocess.run(["cargo", "build", "--locked", "--manifest-path", str(manifest), "--features", "macos-custody-parent-acceptance", "--example", "native-custody-parent"], env=env, check=True)
    parent = root / "native-custody-parent"
    shutil.copy2(build / "debug/examples/native-custody-parent", parent)
    front.command(["/usr/bin/codesign", "--force", "--sign", "-", str(parent)])
    env["COLOSSUS_TLS_BROKER_PARENT_CDHASH"] = front.code_hash(parent)
    env["COLOSSUS_TLS_BROKER_MAC_PACKAGE_TRUST"] = str(root / "compiler-trust.json")
    subprocess.run(["cargo", "build", "--locked", "--manifest-path", str(manifest), "--features", "macos-native-custody-acceptance", "--bin", "colossus-native-custody-acceptance"], env=env, check=True)
    broker = root / "colossus-native-custody-acceptance"
    shutil.copy2(build / "debug/colossus-native-custody-acceptance", broker)
    front.command(["/usr/bin/codesign", "--force", "--sign", "-", str(broker)])
    descriptor = {"schema_version": 1, "purpose": PURPOSE, "production_accepted": False, "cef_artifact_receipt": False, "package_root": str(root), "app_bundle": prepared["package"],
                  "broker_cdhash": front.code_hash(broker), "policy_sha256": prepared["policy_sha256"], "prepared_sha256": front.digest(root / "prepared.json"), "native_result_sha256": front.digest(root / "native-result.json"), "inputs_sha256": front.digest(inputs / "identity.json")}
    payload = json.dumps(descriptor).encode()
    private_write(root / "native-custody-acceptance.json", payload)
    bundle = {"format_version": 1, "name": PURPOSE, "version": "0.1.0", "publisher": PUBLISHER, "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat().replace("+00:00", "Z"), "source_revision": None,
              "files": [{"path": "native-custody-acceptance.json", "size": len(payload), "sha256": hashlib.sha256(payload).hexdigest()}], "signatures": []}
    signature = signer.sign(json.dumps(bundle, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode())
    bundle["signatures"].append({"algorithm": "ed25519", "key_id": key_id, "signature": base64.b64encode(signature).decode()})
    private_write(root / "native-custody-acceptance.bundle.json", json.dumps(bundle).encode())
    # No signing seed is persisted. Public compiler trust is the only retained
    # package signer input; neither broker nor browser receives CA/client keys.
    launch = {"schema_version": 1, "purpose": PURPOSE, "production_accepted": False, "cef_artifact_receipt": False, "parent_sha256": front.digest(parent), "parent_cdhash": front.code_hash(parent), "broker_sha256": front.digest(broker), "broker_cdhash": front.code_hash(broker), "command": [str(parent), str(root)]}
    source_names = (
        "pki/mac/owned_identity.h", "pki/mac/owned_identity.mm",
        "pki/mac/identity_bridge.h", "pki/mac/identity_bridge.mm",
        "pki/mac/src/callbacks.rs", "pki/mac/src/ffi.rs", "pki/mac/src/owner.rs",
        "pki/mac/src/signing.rs", "pki/mac/src/custody.rs",
        "pki/mac/src/custody/journal.rs", "pki/mac/src/custody/journal/model.rs",
        "pki/mac/src/custody/platform.rs", "pki/mac/src/custody/package.rs",
        "pki/mac/src/custody/package/acceptance.rs", "pki/mac/src/custody/package/inventory.rs",
        "pki/mac/src/custody/package/proof.rs", "pki/mac/custody_boundary.h", "pki/mac/custody_boundary.mm",
        "pki/mac/Cargo.toml", "pki/mac/build.rs", "pki/mac/build_custody.rs",
        "tls-broker/Cargo.toml", "tls-broker/Cargo.lock", "tls-broker/build.rs",
        "tls-broker/src/bin/native_custody_acceptance.rs",
        "tls-broker/src/bin/native_custody_acceptance/enrollment.rs",
        "tls-broker/examples/native_custody_parent.rs",
        "tls-broker/examples/native_parent_acceptance/child.rs",
        "tls-broker/examples/native_parent_acceptance/tls.rs",
        "tls-broker/examples/native_custody_parent/diagnostics.rs",
        "tls-broker/examples/native_custody_parent/journal.rs",
    )
    launch["native_identity_source_sha256"] = {"native/browser/" + name: front.digest(front.SOURCE / name) for name in source_names}
    for name in ("crates/colossus-darwin-process/src/identity.rs", "crates/colossus-darwin-process/Cargo.toml"):
        launch["native_identity_source_sha256"][name] = front.digest(front.SOURCE.parent.parent / name)
    private_write(root / "native-custody-launch.json", json.dumps(launch, indent=2).encode())
    return root / "native-custody-launch.json"

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--rebuild-unlaunched", action="store_true", help="Rebuild/sign only an unlaunched fixed fixture, preserving old images/receipts and synthetic inputs")
    options = parser.parse_args()
    try:
        print(prepare(options.root, options.rebuild_unlaunched))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("Native custody preparation failed; partial state retained: " + str(error))
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
