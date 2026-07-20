"""Bounded decoding for canonical Colossus rich gRPC errors."""

from __future__ import annotations

import importlib
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Literal, Protocol

_STATUS_METADATA_KEY = "grpc-status-details-bin"
_COLOSSUS_ERROR_TYPE_URL = "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail"
_MAX_METADATA_ENTRIES = 64
_MAX_STATUS_BYTES = 64 * 1024
_MAX_STATUS_MESSAGE_BYTES = 1024
_MAX_DETAILS = 16
_MAX_DETAIL_BYTES = 16 * 1024
_MAX_REASON_BYTES = 128
_MAX_REQUEST_ID_BYTES = 128
_MAX_VIOLATIONS = 32
_MAX_VIOLATION_FIELD_BYTES = 256
_MAX_VIOLATION_DESCRIPTION_BYTES = 1024
_MAX_DURATION_SECONDS = 315_576_000_000

ErrorOutcomeCertainty = Literal["known", "unknown"]
_MetadataValue = str | bytes
_Metadata = Iterable[tuple[str, _MetadataValue]]


class _RpcErrorLike(Protocol):
    def trailing_metadata(self) -> _Metadata | None: ...

    def code(self) -> object: ...


@dataclass(frozen=True, slots=True)
class ColossusFieldViolation:
    field: str
    description: str


@dataclass(frozen=True, slots=True)
class ColossusRetryAfter:
    seconds: int
    nanos: int


@dataclass(frozen=True, slots=True)
class ColossusRpcError:
    """A bounded, transport-independent view of a Colossus rich gRPC error.

    ``retryable`` is informational. The SDK never automatically retries
    effectful calls, and an ``unknown`` outcome must be reconciled against
    durable state before a caller considers replaying the operation.
    """

    code: int
    message: str
    reason: str
    request_id: str
    retryable: bool
    retry_after: ColossusRetryAfter | None
    outcome_certainty: ErrorOutcomeCertainty
    violations: tuple[ColossusFieldViolation, ...]


def _utf8_length(value: str) -> int:
    return len(value.encode("utf-8"))


def _transport_code(error: _RpcErrorLike) -> int | None:
    try:
        code = error.code()
    except Exception:
        return None

    if isinstance(code, int) and not isinstance(code, bool):
        return code
    value = getattr(code, "value", None)
    if (
        isinstance(value, tuple)
        and len(value) >= 1
        and isinstance(value[0], int)
        and not isinstance(value[0], bool)
    ):
        return value[0]
    return None


def _status_bytes(error: _RpcErrorLike) -> bytes | None:
    try:
        metadata = error.trailing_metadata()
    except Exception:
        return None
    if metadata is None:
        return None

    values: list[bytes] = []
    try:
        for index, item in enumerate(metadata):
            if index >= _MAX_METADATA_ENTRIES:
                return None
            key, value = item
            if key == _STATUS_METADATA_KEY:
                if not isinstance(value, bytes):
                    return None
                values.append(value)
    except Exception:
        return None

    if len(values) != 1:
        return None
    return values[0]


def decode_colossus_rpc_error(error: _RpcErrorLike) -> ColossusRpcError | None:
    """Decode one canonical ``grpc-status-details-bin`` trailer.

    Malformed, oversized, duplicated, non-Colossus, or transport-code-mismatched
    details return ``None``. This helper does not log content, resolve ``Any``
    type URLs, or retry an RPC.
    """

    transport_code = _transport_code(error)
    raw_status = _status_bytes(error)
    if (
        transport_code is None
        or transport_code < 1
        or transport_code > 16
        or raw_status is None
        or not raw_status
        or len(raw_status) > _MAX_STATUS_BYTES
    ):
        return None

    status_pb2 = importlib.import_module("google.rpc.status_pb2")
    status = status_pb2.Status()
    try:
        status.ParseFromString(raw_status)
    except Exception:
        return None
    if (
        status.code != transport_code
        or _utf8_length(status.message) > _MAX_STATUS_MESSAGE_BYTES
        or len(status.details) > _MAX_DETAILS
    ):
        return None

    matching_details = [
        detail for detail in status.details if detail.type_url == _COLOSSUS_ERROR_TYPE_URL
    ]
    if len(matching_details) != 1:
        return None
    packed_detail = matching_details[0]
    if len(packed_detail.value) > _MAX_DETAIL_BYTES:
        return None

    common_pb2 = importlib.import_module("colossus.api.v1alpha1.common_pb2")
    detail = common_pb2.ColossusErrorDetail()
    try:
        detail.ParseFromString(packed_detail.value)
    except Exception:
        return None

    if detail.outcome_certainty == 1:
        outcome: ErrorOutcomeCertainty = "known"
    elif detail.outcome_certainty == 2:
        outcome = "unknown"
    else:
        return None

    if (
        _utf8_length(detail.reason) > _MAX_REASON_BYTES
        or _utf8_length(detail.request_id) > _MAX_REQUEST_ID_BYTES
        or len(detail.violations) > _MAX_VIOLATIONS
    ):
        return None

    violations: list[ColossusFieldViolation] = []
    for violation in detail.violations:
        if (
            _utf8_length(violation.field) > _MAX_VIOLATION_FIELD_BYTES
            or _utf8_length(violation.description) > _MAX_VIOLATION_DESCRIPTION_BYTES
        ):
            return None
        violations.append(
            ColossusFieldViolation(
                field=violation.field,
                description=violation.description,
            )
        )

    retry_after: ColossusRetryAfter | None = None
    if detail.HasField("retry_after"):
        seconds = detail.retry_after.seconds
        nanos = detail.retry_after.nanos
        if seconds < 0 or seconds > _MAX_DURATION_SECONDS or nanos < 0 or nanos > 999_999_999:
            return None
        retry_after = ColossusRetryAfter(seconds=seconds, nanos=nanos)

    return ColossusRpcError(
        code=status.code,
        message=status.message,
        reason=detail.reason,
        request_id=detail.request_id,
        retryable=detail.retryable,
        retry_after=retry_after,
        outcome_certainty=outcome,
        violations=tuple(violations),
    )
