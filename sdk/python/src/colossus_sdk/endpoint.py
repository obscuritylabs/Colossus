"""Strict endpoint descriptor and public certificate pin validation."""

from __future__ import annotations

import hashlib
import hmac
import json
import re
import ssl
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any
from urllib.parse import SplitResult, urlsplit

from cryptography import x509

_PIN = re.compile(r"^[0-9a-f]{64}$")
_INSTANCE_ID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
_ENDPOINT = re.compile(r"^https://(?:127\.0\.0\.1|\[::1\]):[1-9][0-9]{0,4}/?$")
_CERTIFICATE_PEM = re.compile(
    r"\A\s*-----BEGIN CERTIFICATE-----\r?\n"
    r"[A-Za-z0-9+/=\r\n]+"
    r"-----END CERTIFICATE-----\s*\Z"
)
_ALLOWED_FIELDS = frozenset(
    {
        "schema_version",
        "api_version",
        "instance_id",
        "endpoint",
        "pid",
        "certificate_sha256",
    }
)


def _bounded_string(
    source: Mapping[str, Any],
    field: str,
    maximum: int,
) -> str:
    value = source.get(field)
    if (
        not isinstance(value, str)
        or not value
        or len(value) > maximum
        or any(ord(character) < 32 or ord(character) == 127 for character in value)
    ):
        raise ValueError(f"{field} must be a non-empty bounded string")
    return value


def _validate_endpoint(value: str) -> tuple[SplitResult, str]:
    if _ENDPOINT.fullmatch(value) is None:
        raise ValueError("endpoint must be a canonical credential-free https literal-loopback URL")
    try:
        endpoint = urlsplit(value)
        port = endpoint.port
    except ValueError as error:
        raise ValueError("endpoint is malformed") from error

    if (
        endpoint.scheme != "https"
        or endpoint.username is not None
        or endpoint.password is not None
        or endpoint.query
        or endpoint.fragment
        or endpoint.path not in {"", "/"}
    ):
        raise ValueError(
            "endpoint must be a credential-free https URL without path, query, or fragment"
        )
    if endpoint.hostname not in {"127.0.0.1", "::1"}:
        raise ValueError("endpoint host must be a literal loopback address")
    if port is None or not 1 <= port <= 65535:
        raise ValueError("endpoint must contain a valid explicit port")

    target = f"[::1]:{port}" if endpoint.hostname == "::1" else f"127.0.0.1:{port}"
    return endpoint, target


@dataclass(frozen=True, slots=True)
class EndpointDescriptor:
    """Credential-free owner-readable connection metadata."""

    schema_version: int
    api_version: str
    instance_id: str
    endpoint: str
    target: str
    pid: int
    certificate_sha256: str

    def validated(self) -> EndpointDescriptor:
        """Return canonical, revalidated connection metadata."""

        validated = EndpointDescriptor.from_json(
            {
                "schema_version": self.schema_version,
                "api_version": self.api_version,
                "instance_id": self.instance_id,
                "endpoint": self.endpoint,
                "pid": self.pid,
                "certificate_sha256": self.certificate_sha256,
            }
        )
        if validated.target != self.target:
            raise ValueError("endpoint descriptor is inconsistent")
        return validated

    def validate(self) -> None:
        """Revalidate manually constructed metadata at the connection boundary."""

        self.validated()

    @classmethod
    def from_json(cls, value: str | bytes | Mapping[str, Any]) -> EndpointDescriptor:
        decoded: Any
        if isinstance(value, (str, bytes)):
            if len(value) > 4096:
                raise ValueError("endpoint descriptor exceeds the size limit")
            try:
                decoded = json.loads(value)
            except (json.JSONDecodeError, UnicodeDecodeError):
                raise ValueError("endpoint descriptor is invalid JSON") from None
        else:
            decoded = value
        if not isinstance(decoded, Mapping):
            raise ValueError("endpoint descriptor must be a JSON object")

        unknown = set(decoded) - _ALLOWED_FIELDS
        if unknown:
            raise ValueError("endpoint descriptor contains an unsupported field")
        if type(decoded.get("schema_version")) is not int or decoded["schema_version"] != 1:
            raise ValueError("unsupported endpoint descriptor schema_version")

        api_version = _bounded_string(decoded, "api_version", 64)
        if api_version != "colossus.api.v1alpha1":
            raise ValueError("unsupported endpoint descriptor api_version")
        instance_id = _bounded_string(decoded, "instance_id", 128)
        if (
            _INSTANCE_ID.fullmatch(instance_id) is None
            or instance_id == "00000000-0000-0000-0000-000000000000"
        ):
            raise ValueError("instance_id must be a canonical non-nil UUID")
        endpoint_value = _bounded_string(decoded, "endpoint", 256)
        pid = decoded.get("pid")
        if type(pid) is not int or not 1 <= pid <= 0xFFFFFFFF:
            raise ValueError("pid must be a nonzero unsigned 32-bit integer")
        pin = _bounded_string(decoded, "certificate_sha256", 64)
        if _PIN.fullmatch(pin) is None:
            raise ValueError("certificate_sha256 must be 64 lowercase hexadecimal digits")

        _endpoint, target = _validate_endpoint(endpoint_value)

        return cls(
            schema_version=1,
            api_version=api_version,
            instance_id=instance_id,
            endpoint=endpoint_value,
            target=target,
            pid=pid,
            certificate_sha256=pin,
        )


def _single_certificate_der(leaf_certificate_pem: str | bytes) -> bytes:
    if len(leaf_certificate_pem) > 65536:
        raise ValueError("leaf certificate exceeds the size limit")
    if isinstance(leaf_certificate_pem, bytes):
        try:
            pem = leaf_certificate_pem.decode("ascii")
        except UnicodeDecodeError as error:
            raise ValueError("leaf certificate PEM must be ASCII") from error
    else:
        pem = leaf_certificate_pem
    if _CERTIFICATE_PEM.fullmatch(pem) is None:
        raise ValueError("exactly one public leaf certificate is required")
    try:
        der = ssl.PEM_cert_to_DER_cert(pem)
    except ValueError as error:
        raise ValueError("leaf certificate is not valid PEM") from error
    try:
        certificate = x509.load_der_x509_certificate(der)
    except ValueError as error:
        raise ValueError("leaf certificate is invalid") from error
    try:
        constraints = certificate.extensions.get_extension_for_class(x509.BasicConstraints).value
    except x509.ExtensionNotFound as error:
        raise ValueError(
            "endpoint identity certificate must declare BasicConstraints CA=false"
        ) from error
    if constraints.ca:
        raise ValueError("endpoint identity certificate must declare BasicConstraints CA=false")
    return der


def certificate_sha256(leaf_certificate_pem: str | bytes) -> str:
    """Return the lowercase SHA-256 digest of the leaf's DER encoding."""

    return hashlib.sha256(_single_certificate_der(leaf_certificate_pem)).hexdigest()


def assert_pinned_leaf_certificate(
    descriptor: EndpointDescriptor,
    leaf_certificate_pem: str | bytes,
    expected_instance_id: str,
    expected_certificate_sha256: str,
) -> None:
    """Verify discovery and the public leaf against an independent trust anchor."""

    if (
        _INSTANCE_ID.fullmatch(expected_instance_id) is None
        or expected_instance_id == "00000000-0000-0000-0000-000000000000"
    ):
        raise ValueError("independently provisioned instance ID must be a canonical non-nil UUID")
    if _PIN.fullmatch(expected_certificate_sha256) is None:
        raise ValueError(
            "independently provisioned certificate pin must be 64 lowercase hexadecimal digits"
        )
    if not hmac.compare_digest(descriptor.instance_id, expected_instance_id):
        raise ValueError(
            "endpoint descriptor instance ID does not match the independently provisioned identity"
        )
    if not hmac.compare_digest(
        descriptor.certificate_sha256,
        expected_certificate_sha256,
    ):
        raise ValueError(
            "endpoint descriptor certificate pin does not match the independently provisioned pin"
        )
    if not hmac.compare_digest(
        certificate_sha256(leaf_certificate_pem),
        expected_certificate_sha256,
    ):
        raise ValueError("leaf certificate does not match the independently provisioned pin")
