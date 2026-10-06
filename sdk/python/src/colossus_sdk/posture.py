"""Closed, bounded runtime configuration metadata; never execution authority."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from enum import Enum
from typing import Any, TypeVar


class PolicyProvenance(str, Enum):
    RUNTIME_REPORTED = "runtime_reported"


class PolicySandboxBackend(str, Enum):
    NATIVE = "native"
    WINDOWS_JOB = "windows_job"
    OCI = "oci"
    EXTERNAL = "external"
    DANGER_FULL_ACCESS = "danger_full_access"
    UNKNOWN = "unknown"


class PolicyApprovalMode(str, Enum):
    DENY = "deny"
    ASK = "ask"
    RISK_AUTO = "risk_auto"
    DANGER_AUTO = "danger_auto"
    UNKNOWN = "unknown"


class PolicyFindingCode(str, Enum):
    EPHEMERAL = "storage.ephemeral"
    PLAINTEXT = "storage.plaintext"
    FULL_ACCESS = "sandbox.danger_full_access"
    SENSITIVE_JOURNAL = "observability.sensitive_journal_payloads"
    PLAINTEXT_OAUTH = "credentials.mcp_oauth_plaintext"


class PolicyFindingSeverity(str, Enum):
    WARNING = "warning"


class PolicyTelemetryProvenance(str, Enum):
    UNAVAILABLE = "unavailable"


@dataclass(frozen=True, slots=True)
class PolicyModelLabel:
    profile: str
    label: str


@dataclass(frozen=True, slots=True)
class PolicyFinding:
    code: PolicyFindingCode
    severity: PolicyFindingSeverity


@dataclass(frozen=True, slots=True)
class PolicyTelemetry:
    provenance: PolicyTelemetryProvenance
    denied_requests: None = None
    approval_requests: None = None
    outcome_unknown_runs: None = None


@dataclass(frozen=True, slots=True)
class RuntimePolicyPosture:
    schema_version: int
    provenance: PolicyProvenance
    fingerprint: str
    configuration_revision: int | None
    access_profile: str
    sandbox_backend: PolicySandboxBackend
    sandbox_profile: str
    boundary_acknowledged: bool
    approval_mode: PolicyApprovalMode
    allowed_roles: tuple[str, ...]
    allowed_tools: tuple[str, ...]
    capabilities: tuple[str, ...]
    models: tuple[PolicyModelLabel, ...]
    findings: tuple[PolicyFinding, ...]
    telemetry: PolicyTelemetry


_INVALID = "Invalid bounded runtime policy metadata"
_IDENTIFIER = re.compile(r"[A-Za-z0-9._-]{1,128}\Z", re.ASCII)
_EnumType = TypeVar("_EnumType", bound=Enum)


def _pairs(items: list[tuple[str, Any]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in items:
        if key in result:
            raise ValueError(_INVALID)
        result[key] = value
    return result


def _object(value: object, required: set[str], optional: set[str] | None = None) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(_INVALID)
    allowed = required | (optional or set())
    if not required <= value.keys() or not value.keys() <= allowed:
        raise ValueError(_INVALID)
    return value


def _identifier(value: object) -> str:
    if not isinstance(value, str) or _IDENTIFIER.fullmatch(value) is None:
        raise ValueError(_INVALID)
    return value


def _list(value: object, maximum: int) -> tuple[str, ...]:
    if not isinstance(value, list) or len(value) > maximum:
        raise ValueError(_INVALID)
    items = tuple(_identifier(item) for item in value)
    if any(left >= right for left, right in zip(items, items[1:])):
        raise ValueError(_INVALID)
    return items


def _enum(value: object, kind: type[_EnumType]) -> _EnumType:
    if not isinstance(value, str):
        raise ValueError(_INVALID)
    return kind(value)


def _reject_number(_value: str) -> None:
    raise ValueError(_INVALID)


def decode_runtime_policy_posture(payload: bytes) -> RuntimePolicyPosture:
    """Reject unknown/duplicate fields, invalid enums, oversized and private metadata."""
    if not isinstance(payload, bytes) or not 0 < len(payload) <= 65_536:
        raise ValueError(_INVALID)
    try:
        raw: object = json.loads(
            payload.decode("utf-8", errors="strict"),
            object_pairs_hook=_pairs,
            parse_float=_reject_number,
            parse_constant=_reject_number,
        )
        p = _object(
            raw,
            {
                "schema_version",
                "provenance",
                "fingerprint",
                "access_profile",
                "sandbox_backend",
                "sandbox_profile",
                "boundary_acknowledged",
                "approval_mode",
                "allowed_roles",
                "allowed_tools",
                "capabilities",
                "models",
                "findings",
                "telemetry",
            },
            {"configuration_revision"},
        )
        if type(p["schema_version"]) is not int or p["schema_version"] != 1:
            raise ValueError(_INVALID)
        if (
            not isinstance(p["fingerprint"], str)
            or re.fullmatch(r"[0-9a-f]{64}", p["fingerprint"]) is None
        ):
            raise ValueError(_INVALID)
        revision = p.get("configuration_revision")
        if revision is not None and (type(revision) is not int or not 0 <= revision <= 2**64 - 1):
            raise ValueError(_INVALID)
        if type(p["boundary_acknowledged"]) is not bool:
            raise ValueError(_INVALID)
        if not isinstance(p["models"], list) or len(p["models"]) > 64:
            raise ValueError(_INVALID)
        if not isinstance(p["findings"], list) or len(p["findings"]) > 16:
            raise ValueError(_INVALID)
        models = []
        for value in p["models"]:
            model = _object(value, {"profile", "label"})
            models.append(
                PolicyModelLabel(_identifier(model["profile"]), _identifier(model["label"]))
            )
        findings = []
        for value in p["findings"]:
            finding = _object(value, {"code", "severity"})
            findings.append(
                PolicyFinding(
                    _enum(finding["code"], PolicyFindingCode),
                    _enum(finding["severity"], PolicyFindingSeverity),
                )
            )
        telemetry = _object(
            p["telemetry"],
            {"provenance"},
            {
                "denied_requests",
                "approval_requests",
                "outcome_unknown_runs",
            },
        )
        if any(
            telemetry.get(key) is not None
            for key in (
                "denied_requests",
                "approval_requests",
                "outcome_unknown_runs",
            )
        ):
            raise ValueError(_INVALID)
        return RuntimePolicyPosture(
            schema_version=1,
            provenance=_enum(p["provenance"], PolicyProvenance),
            fingerprint=p["fingerprint"],
            configuration_revision=revision,
            access_profile=_identifier(p["access_profile"]),
            sandbox_backend=_enum(p["sandbox_backend"], PolicySandboxBackend),
            sandbox_profile=_identifier(p["sandbox_profile"]),
            boundary_acknowledged=p["boundary_acknowledged"],
            approval_mode=_enum(p["approval_mode"], PolicyApprovalMode),
            allowed_roles=_list(p["allowed_roles"], 64),
            allowed_tools=_list(p["allowed_tools"], 256),
            capabilities=_list(p["capabilities"], 256),
            models=tuple(models),
            findings=tuple(findings),
            telemetry=PolicyTelemetry(_enum(telemetry["provenance"], PolicyTelemetryProvenance)),
        )
    except (ValueError, TypeError, OverflowError, RecursionError):
        raise ValueError(_INVALID) from None
