"""Mint unique disposable CA/server/client fixtures without changing OS trust."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import secrets
import shutil
import ssl
import subprocess
import uuid

from pki_fixture_private import private_directory, private_write


def generate(directory: Path) -> dict:
    """Create private keys and public metadata in a new owner-private directory."""
    openssl = shutil.which("openssl")
    if openssl is None:
        raise ValueError("OpenSSL is required for disposable PKI fixtures")
    private_directory(directory)
    fixture_id = str(uuid.uuid4())
    password = secrets.token_hex(24)
    private_write(directory / "passphrase.txt", password.encode("ascii"))
    environment = {key: os.environ[key] for key in
                   ("PATH", "SYSTEMROOT", "SystemRoot", "WINDIR", "TEMP", "TMP")
                   if key in os.environ}
    environment["LANG"] = "C"

    def command(*arguments: str) -> None:
        try:
            result = subprocess.run([openssl, *arguments], cwd=directory,
                env=environment, capture_output=True, timeout=15, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ValueError("disposable PKI generation could not complete") from error
        if result.returncode != 0 or len(result.stdout) + len(result.stderr) > 65536:
            # Tool diagnostics, selected paths, and key bytes are not evidence.
            raise ValueError("disposable PKI generation could not complete")

    def key(name: str) -> None:
        command("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
                "-out", f"{name}.key")

    for name in ("ca", "other_ca"):
        key(name)
        command("req", "-new", "-x509", "-sha256", "-days", "3", "-key", f"{name}.key",
                "-out", f"{name}.pem", "-subj", f"/CN=Colossus fixture {name} {fixture_id}",
                "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0",
                "-addext", "keyUsage=critical,keyCertSign,cRLSign")
    private_write(directory / "ca.der", ssl.PEM_cert_to_DER_cert(
                  (directory / "ca.pem").read_text("ascii")))
    private_write(directory / "ca-index.txt", b"")
    private_write(directory / "ca-serial.txt", b"1001\n")
    private_write(directory / "expired-ca.conf", b"[ca]\ndefault_ca=fixture\n[fixture]\n"
        b"database=ca-index.txt\nnew_certs_dir=.\ncertificate=ca.pem\nprivate_key=ca.key\n"
        b"serial=ca-serial.txt\ndefault_md=sha256\npolicy=identity\nunique_subject=no\n"
        b"[identity]\ncommonName=supplied\n")

    certificate_names = ("server", "untrusted_server", "wrong_host_server", "expired_server",
                         "client", "alternate_client", "wrong_issuer_client", "expired_client",
                         "wrong_usage_client")
    for index, name in enumerate(certificate_names, 1):
        key(name)
        client = "client" in name
        usage = "clientAuth" if client and name != "wrong_usage_client" else "serverAuth"
        issuer = "other_ca" if name in ("untrusted_server", "wrong_issuer_client") else "ca"
        extension = "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\n"
        extension += f"extendedKeyUsage={usage}\n"
        if not client:
            extension += "subjectAltName=" + ("DNS:wrong.invalid\n" if
                         name == "wrong_host_server" else "IP:127.0.0.1,DNS:localhost\n")
        private_write(directory / f"{name}.ext", extension.encode("ascii"))
        command("req", "-new", "-key", f"{name}.key", "-out", f"{name}.csr",
                "-subj", f"/CN=Colossus {name} {fixture_id.replace('-', '')}")
        if name.startswith("expired_"):
            command("ca", "-batch", "-notext", "-config", "expired-ca.conf",
                    "-in", f"{name}.csr", "-out", f"{name}.pem", "-extfile", f"{name}.ext",
                    "-startdate", "20200101000000Z", "-enddate", "20200102000000Z")
        else:
            command("x509", "-req", "-sha256", "-in", f"{name}.csr", "-CA", f"{issuer}.pem",
                    "-CAkey", f"{issuer}.key", "-set_serial", str(index), "-days", "3",
                    "-extfile", f"{name}.ext", "-out", f"{name}.pem")
        if client:
            command("pkcs12", "-export", "-inkey", f"{name}.key", "-in", f"{name}.pem",
                    "-name", f"Colossus fixture {name} {fixture_id}", "-out", f"{name}.pfx",
                    "-passout", "file:passphrase.txt")

    fingerprints = {}
    for name in ("ca", *certificate_names):
        der = ssl.PEM_cert_to_DER_cert((directory / f"{name}.pem").read_text("ascii"))
        if name != "ca":
            private_write(directory / f"{name}.der", der)
        fingerprints[name] = hashlib.sha256(der).hexdigest()
    # A certificate bag in an identity bundle is deliberately not reviewed CA
    # trust. The native NSS probe verifies import cannot implicitly trust it.
    command("pkcs12", "-export", "-inkey", "client.key", "-in", "client.pem",
            "-certfile", "ca.pem", "-name", f"Colossus fixture chain {fixture_id}",
            "-out", "client_with_ca.pfx", "-passout", "file:passphrase.txt")
    # The parent was private before OpenSSL allocated files; enforce file modes
    # for Unix too. Windows files inherit only the new owner's protected DACL.
    if os.name != "nt":
        for path in directory.iterdir():
            path.chmod(0o600)
    return {"schema_version": 1, "fixture_id": fixture_id, "fingerprints_sha256": fingerprints,
            "ca_file": "ca.der", "identity_file": "client.pfx",
            "alternate_identity_file": "alternate_client.pfx", "passphrase_file": "passphrase.txt",
            "changes_os_trust": False, "production_acceptance": False}
