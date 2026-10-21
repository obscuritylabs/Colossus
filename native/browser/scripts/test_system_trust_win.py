#!/usr/bin/env python3
"""Run native CryptoAPI diagnostic fixtures without installing any certificate.

All trust anchors and CRLs live in exclusive memory stores in the native test.
The loopback trap proves diagnostic validation never retrieves AIA or CRL URLs.
This is not an installed custom-CA Chromium handshake or key-custody acceptance.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import threading
import uuid

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import AuthorityInformationAccessOID, ExtendedKeyUsageOID, NameOID

from pki_fixture_private import private_directory, private_write


def run(executable: Path, directory: Path) -> None:
    private_directory(directory)
    requests = []

    class Trap(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(True)
            self.send_error(404)

        def log_message(self, *_):
            pass

    trap = ThreadingHTTPServer(("127.0.0.1", 0), Trap)
    thread = threading.Thread(target=trap.serve_forever, daemon=True)
    thread.start()
    try:
        now = datetime.now(timezone.utc)
        fixture = uuid.uuid4().hex
        retrieval = f"http://127.0.0.1:{trap.server_port}/forbidden"

        def key():
            return rsa.generate_private_key(public_exponent=65537, key_size=2048)

        def certificate(name, private_key, issuer=None, issuer_key=None, *, ca=False,
                        expired=False, client=False):
            subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, f"{name}-{fixture}")])
            builder = (x509.CertificateBuilder().subject_name(subject)
                .issuer_name(issuer.subject if issuer else subject)
                .public_key(private_key.public_key()).serial_number(x509.random_serial_number())
                .not_valid_before(now - timedelta(days=20))
                .not_valid_after(now + timedelta(days=-1 if expired else 20))
                .add_extension(x509.BasicConstraints(ca=ca, path_length=None), True)
                .add_extension(x509.KeyUsage(digital_signature=True, content_commitment=False,
                    key_encipherment=not ca, data_encipherment=False, key_agreement=False,
                    key_cert_sign=ca, crl_sign=ca, encipher_only=None, decipher_only=None), True)
                .add_extension(x509.SubjectKeyIdentifier.from_public_key(private_key.public_key()), False))
            if issuer:
                builder = builder.add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(
                    issuer_key.public_key()), False)
                builder = builder.add_extension(x509.AuthorityInformationAccess([
                    x509.AccessDescription(AuthorityInformationAccessOID.CA_ISSUERS,
                        x509.UniformResourceIdentifier(retrieval))]), False)
                builder = builder.add_extension(x509.CRLDistributionPoints([
                    x509.DistributionPoint([x509.UniformResourceIdentifier(retrieval)],
                        None, None, None)]), False)
            if not ca:
                builder = builder.add_extension(x509.SubjectAlternativeName([
                    x509.DNSName("pki-fixture.invalid")]), False)
                builder = builder.add_extension(x509.ExtendedKeyUsage([
                    ExtendedKeyUsageOID.CLIENT_AUTH if client else ExtendedKeyUsageOID.SERVER_AUTH]), False)
            return builder.sign(issuer_key or private_key, hashes.SHA256())

        root_key, intermediate_key = key(), key()
        root = certificate("root", root_key, ca=True)
        intermediate = certificate("intermediate", intermediate_key, root, root_key, ca=True)
        material = {"root": root, "intermediate": intermediate}
        for name in ("valid", "expired", "revoked", "wrong_usage"):
            material[name] = certificate(name, key(), intermediate, intermediate_key,
                expired=name == "expired", client=name == "wrong_usage")
        other_key = key()
        other = certificate("other", other_key, ca=True)
        material["untrusted"] = certificate("untrusted", key(), other, other_key)
        for name, cert in material.items():
            private_write(directory / f"{name}.der", cert.public_bytes(serialization.Encoding.DER))
        for name, issuer, issuer_key, revoked in (
                ("root", root, root_key, []),
                ("intermediate", intermediate, intermediate_key, [material["revoked"]])):
            builder = (x509.CertificateRevocationListBuilder().issuer_name(issuer.subject)
                .last_update(now - timedelta(hours=1)).next_update(now + timedelta(days=1))
                .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(
                    issuer_key.public_key()), False))
            for cert in revoked:
                builder = builder.add_revoked_certificate(x509.RevokedCertificateBuilder()
                    .serial_number(cert.serial_number).revocation_date(now - timedelta(minutes=1)).build())
            private_write(directory / f"{name}.crl", builder.sign(issuer_key, hashes.SHA256())
                .public_bytes(serialization.Encoding.DER))
        # Private key objects exist only in this Python process; no keys or
        # personal OS-store credentials are written to disk or subprocess input.
        result = subprocess.run([str(executable), str(directory)], capture_output=True,
            timeout=30, check=False)
        if len(result.stdout) + len(result.stderr) > 16 * 1024:
            raise RuntimeError("native trust fixture exceeded its output bound")
        print(result.stdout.decode("ascii", errors="replace"), end="")
        print(result.stderr.decode("ascii", errors="replace"), end="")
        if result.returncode:
            raise RuntimeError("native trust fixture failed")
        if requests:
            raise RuntimeError("native trust diagnostics made a forbidden network retrieval")
        print("PASS zero AIA/CRL network retrievals; no OS trust or private key changes")
    finally:
        trap.shutdown()
        trap.server_close()
        thread.join(timeout=2)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    arguments = parser.parse_args()
    run(arguments.executable.resolve(strict=True), arguments.directory.absolute())
