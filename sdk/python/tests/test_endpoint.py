from __future__ import annotations

import ipaddress
import unittest
from dataclasses import replace
from datetime import datetime, timedelta, timezone

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

from colossus_sdk.endpoint import (
    EndpointDescriptor,
    assert_pinned_leaf_certificate,
    certificate_sha256,
)

LEAF_PEM = """-----BEGIN CERTIFICATE-----
MIIB1DCCAXqgAwIBAgIUN6hM/NLzLJT2R1GSR2y4mKobn6wwCgYIKoZIzj0EAwIw
HDEaMBgGA1UEAwwRQ29sb3NzdXMtU0RLLVRlc3QwHhcNMjYwNzE5MTY1NDU4WhcN
MzYwNzE2MTY1NDU4WjAcMRowGAYDVQQDDBFDb2xvc3N1cy1TREstVGVzdDBZMBMG
ByqGSM49AgEGCCqGSM49AwEHA0IABOk9QOGFQZSF+hlCY9tOz0Ob8Aca9e7RNDi9
9D2kHJS5VKeCSN/8mIs59wT+C3IpyHToSaIZZn6s+/hycQpy7zajgZkwgZYwHQYD
VR0OBBYEFEE/MJGMh+aakko7MGPckqzWUyN8MB8GA1UdIwQYMBaAFEE/MJGMh+aa
kko7MGPckqzWUyN8MCEGA1UdEQQaMBiHBH8AAAGHEAAAAAAAAAAAAAAAAAAAAAEw
DAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCB4AwEwYDVR0lBAwwCgYIKwYBBQUH
AwEwCgYIKoZIzj0EAwIDSAAwRQIgLDKmWGWi+VArQc1vxHAaxvbW0TKk+Jz6Cwnj
B3vteJgCIQDCudF1+zHi2zb9DDOG7S6e+i7kGNR0a8oGuIvFlqJG7w==
-----END CERTIFICATE-----
"""


def self_signed_certificate(basic_constraints: x509.BasicConstraints | None) -> bytes:
    key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Colossus test")])
    now = datetime.now(timezone.utc)
    builder = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(1)
        .not_valid_before(now - timedelta(minutes=1))
        .not_valid_after(now + timedelta(hours=1))
        .add_extension(
            x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]),
            critical=False,
        )
    )
    if basic_constraints is not None:
        builder = builder.add_extension(basic_constraints, critical=True)
    certificate = builder.sign(key, hashes.SHA256())
    return certificate.public_bytes(serialization.Encoding.PEM)


def valid_descriptor() -> dict[str, object]:
    return {
        "schema_version": 1,
        "api_version": "colossus.api.v1alpha1",
        "instance_id": "00000000-0000-4000-8000-000000000001",
        "endpoint": "https://127.0.0.1:43119",
        "pid": 4242,
        "certificate_sha256": "a" * 64,
    }


class EndpointTests(unittest.TestCase):
    def test_accepts_only_pinned_literal_loopback(self) -> None:
        descriptor = EndpointDescriptor.from_json(valid_descriptor())
        self.assertEqual(descriptor.target, "127.0.0.1:43119")
        self.assertEqual(
            descriptor.instance_id,
            "00000000-0000-4000-8000-000000000001",
        )
        self.assertEqual(descriptor.pid, 4242)

    def test_rejects_remote_plaintext_and_credentials(self) -> None:
        for endpoint in (
            "https://example.com:43119",
            "http://127.0.0.1:43119",
            "https://user:pass@127.0.0.1:43119",
            "https://localhost:43119",
        ):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                EndpointDescriptor.from_json({**valid_descriptor(), "endpoint": endpoint})

    def test_descriptor_cannot_contain_a_token(self) -> None:
        with self.assertRaisesRegex(ValueError, "unsupported"):
            EndpointDescriptor.from_json(
                {
                    **valid_descriptor(),
                    "bearer_token": "cls_v1.should-never-be-here",
                }
            )
        with self.assertRaisesRegex(ValueError, "unsupported"):
            EndpointDescriptor.from_json(
                {
                    **valid_descriptor(),
                    "server_name": "localhost",
                }
            )

    def test_descriptor_json_is_bounded_before_parsing(self) -> None:
        with self.assertRaisesRegex(ValueError, "size"):
            EndpointDescriptor.from_json(" " * 4097)

    def test_requires_exact_api_identity_uuid_and_nonzero_pid(self) -> None:
        for changed in (
            {"api_version": "v1"},
            {"instance_id": "not-a-uuid"},
            {"instance_id": "00000000-0000-0000-0000-000000000000"},
            {"pid": 0},
            {"certificate_sha256": "A" * 64},
        ):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                EndpointDescriptor.from_json({**valid_descriptor(), **changed})

    def test_rejects_noncanonical_loopback_spellings(self) -> None:
        for endpoint in (
            "https://127.1:43119",
            "https://127.0.0.1:043119",
            "https://[0:0:0:0:0:0:0:1]:43119",
        ):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                EndpointDescriptor.from_json({**valid_descriptor(), "endpoint": endpoint})

    def test_connection_time_validation_rejects_forged_descriptor(self) -> None:
        descriptor = EndpointDescriptor.from_json(valid_descriptor())
        descriptor.validate()
        with self.assertRaisesRegex(ValueError, "inconsistent"):
            replace(descriptor, target="example.com:443").validate()
        with self.assertRaisesRegex(ValueError, "endpoint"):
            replace(
                descriptor,
                endpoint="https://example.com:443",
                target="example.com:443",
            ).validate()

    def test_public_leaf_must_match_an_independently_provisioned_pin(self) -> None:
        pin = "a1f509c8e6096e1dbdacc7c89cb4a7895ca71d2f2c4b024449e6c2b35f8c5f0c"
        self.assertEqual(certificate_sha256(LEAF_PEM), pin)

        descriptor_json = {**valid_descriptor(), "certificate_sha256": pin}
        descriptor = EndpointDescriptor.from_json(descriptor_json)
        assert_pinned_leaf_certificate(descriptor, LEAF_PEM, descriptor.instance_id, pin)
        with self.assertRaisesRegex(ValueError, "independently provisioned"):
            assert_pinned_leaf_certificate(
                EndpointDescriptor.from_json(valid_descriptor()),
                LEAF_PEM,
                descriptor.instance_id,
                pin,
            )
        with self.assertRaisesRegex(ValueError, "independently provisioned"):
            assert_pinned_leaf_certificate(
                descriptor,
                LEAF_PEM,
                descriptor.instance_id,
                "b" * 64,
            )
        with self.assertRaisesRegex(ValueError, "pin"):
            assert_pinned_leaf_certificate(
                descriptor,
                LEAF_PEM,
                descriptor.instance_id,
                "A" * 64,
            )
        with self.assertRaisesRegex(ValueError, "instance ID"):
            assert_pinned_leaf_certificate(
                descriptor,
                LEAF_PEM,
                "00000000-0000-4000-8000-000000000002",
                pin,
            )
        with self.assertRaisesRegex(ValueError, "exactly one"):
            certificate_sha256(f"{LEAF_PEM}\n{LEAF_PEM}")
        with self.assertRaisesRegex(ValueError, "size"):
            certificate_sha256("A" * 65537)
        with self.assertRaisesRegex(ValueError, "BasicConstraints"):
            certificate_sha256(self_signed_certificate(None))
        with self.assertRaisesRegex(ValueError, "BasicConstraints"):
            certificate_sha256(
                self_signed_certificate(x509.BasicConstraints(ca=True, path_length=0))
            )


if __name__ == "__main__":
    unittest.main()
