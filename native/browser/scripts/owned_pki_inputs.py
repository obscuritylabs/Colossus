"""Prepare fixed synthetic owned-Core PKI inputs; never launch or use OS stores.

Client secrets are written only under the protected broker enrollment directory.
Core's fixture directory receives a synthetic server key and public certificates.
The public registry needs its digest in the independently signed package; this
module alone establishes neither native admission nor acceptance.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import stat
from urllib.parse import urlsplit

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

REGISTRY_PURPOSE = "colossus-native-browser-reviewed-pki-registry"
APPLICATION_ID = "authenticated-app"
CONVERSATION_ID = "conversation"


def private_directory(path: Path) -> None:
    value = path.lstat()
    if (not path.is_absolute() or path.resolve(strict=True) != path or
            not stat.S_ISDIR(value.st_mode) or value.st_uid != os.geteuid() or
            stat.S_IMODE(value.st_mode) != 0o700):
        raise ValueError("fixture requires exact owner-private directories")


def private_write(path: Path, value: bytes) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                         os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(value)
        output.flush()
        os.fsync(output.fileno())


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def workspace_id(workspace: Path) -> str:
    # Runtime's repository_identity hashes the exact canonical UTF-8 path. A
    # fixture path that cannot be represented by that same encoding is rejected.
    return "repo-" + sha256(str(workspace).encode("utf-8", errors="strict"))


def fixed_origin(origin: str) -> None:
    parsed = urlsplit(origin)
    port = parsed.port
    if (parsed.scheme != "https" or parsed.hostname != "127.0.0.1" or
            port is None or port in (0, 443) or parsed.username is not None or
            parsed.password is not None or parsed.path or parsed.query or parsed.fragment or
            origin != f"https://127.0.0.1:{port}"):
        raise ValueError("one canonical numeric fixture HTTPS origin is required")


def certificate(name: str, key, issuer, issuer_key, *, ca: bool, server: bool = False):
    now = datetime.now(timezone.utc)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, name)])
    builder = (x509.CertificateBuilder().subject_name(subject)
        .issuer_name(issuer.subject if issuer else subject).public_key(key.public_key())
        .serial_number(x509.random_serial_number()).not_valid_before(now - timedelta(minutes=5))
        .not_valid_after(now + timedelta(days=2))
        .add_extension(x509.BasicConstraints(ca=ca, path_length=0 if ca else None), critical=True)
        .add_extension(x509.KeyUsage(not ca, False, False, False, False, ca, ca, False, False),
                       critical=True))
    if not ca:
        builder = builder.add_extension(x509.ExtendedKeyUsage([
            ExtendedKeyUsageOID.SERVER_AUTH if server else ExtendedKeyUsageOID.CLIENT_AUTH]),
            critical=False)
    if server:
        builder = builder.add_extension(x509.SubjectAlternativeName([
            x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]), critical=False)
    return builder.sign(issuer_key or key, hashes.SHA256())


def prepare(broker: Path, fixture_inputs: Path, workspace: Path, origin: str) -> dict:
    """Write a new fixed catalog plus inputs. Preserve partial outputs on failure."""
    fixed_origin(origin)
    for directory in (broker, fixture_inputs, workspace):
        private_directory(directory)
    enrollment = broker / "pki-enrollment"
    registry_path = broker / "owned-pki-registry.json"
    protected_names = [enrollment, registry_path, broker / "colossus-tls-identity",
                       broker / "colossus-custody-generation.jsonl"]
    protected_names += [broker / name for name in os.listdir(broker)
                        if name.startswith(".retired-tls-identity-")]
    outputs = [fixture_inputs / name for name in (
        "fixture-ca.der", "fixture-leaf.der", "fixture-key.pk8", "fixture-client.der")]
    if any(os.path.lexists(path) for path in protected_names + outputs):
        raise ValueError("prior fixture outputs or native duties must remain untouched")
    # No runtime repository is adopted as the approved fixture workspace.
    if any(workspace.iterdir()):
        raise ValueError("fixture workspace must be newly allocated and empty")
    owner = workspace_id(workspace)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    ca = certificate("Colossus owned Core fixture CA", ca_key, None, None, ca=True)
    client_key = ec.generate_private_key(ec.SECP256R1())
    client = certificate("Colossus reviewed Core fixture client", client_key, ca, ca_key, ca=False)
    server_key = ec.generate_private_key(ec.SECP256R1())
    server = certificate("Colossus owned Core fixture server", server_key, ca, ca_key,
                         ca=False, server=True)
    ca_der = ca.public_bytes(serialization.Encoding.DER)
    client_der = client.public_bytes(serialization.Encoding.DER)
    password = os.urandom(32).hex().encode("ascii")
    # Encoding only for the disposable native importer fixture. Mac's legacy
    # PKCS12 importer needs this compatible container encoding; certificate and
    # TLS signatures remain SHA256 and no system/default trust changes occur.
    encryption = (serialization.PrivateFormat.PKCS12.encryption_builder().kdf_rounds(50000)
                  .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC)
                  .hmac_hash(hashes.SHA1()).build(password))
    container = pkcs12.serialize_key_and_certificates(b"Colossus reviewed fixture client",
        client_key, client, [ca], encryption)
    ca_id = sha256(b"colossus-fixture-ca-id-v1\0" + os.urandom(32))
    client_id = sha256(b"colossus-fixture-client-id-v1\0" + os.urandom(32))
    metadata = {"schema_version": 1, "purpose": REGISTRY_PURPOSE,
        "cas": [{"id": ca_id, "slot": 0, "sha256": sha256(ca_der), "origins": [origin]}],
        "identities": [{"id": client_id, "slot": 0, "leaf_sha256": sha256(client_der),
            "pkcs12_sha256": sha256(container), "password_sha256": sha256(password),
            "ca_ids": [ca_id], "origins": [origin]}],
        "reviews": [{"workspace_id": owner, "application_id": APPLICATION_ID,
            "scope": {"kind": "conversation", "id": CONVERSATION_ID}, "origins": [origin],
            "ca_ids": [ca_id], "client_id": client_id}]}
    registry_bytes = (json.dumps(metadata, sort_keys=True, separators=(",", ":")) + "\n").encode()
    enrollment.mkdir(mode=0o700)
    for name, value in (("ca-0.der", ca_der), ("client-0.der", client_der),
                        ("client-0.p12", container), ("password-0", password)):
        private_write(enrollment / name, value)
    private_write(registry_path, registry_bytes)
    for name, value in (("fixture-ca.der", ca_der), ("fixture-client.der", client_der),
                        ("fixture-leaf.der", server.public_bytes(serialization.Encoding.DER)),
                        ("fixture-key.pk8", server_key.private_bytes(serialization.Encoding.DER,
                            serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))):
        private_write(fixture_inputs / name, value)
    return {"schema_version": 1, "synthetic_fixture_only": True, "production_accepted": False,
        "native_acceptance": False, "registry_sha256": sha256(registry_bytes),
        "reviewed_leaf_sha256": sha256(client_der), "workspace_id": owner,
        "application_id": APPLICATION_ID, "conversation_id": CONVERSATION_ID,
        "public_ca_der": ca_der}
