from __future__ import annotations

import unittest
from collections.abc import Iterable
from enum import Enum

import grpc
from colossus.api.v1alpha1 import common_pb2
from google.protobuf import any_pb2
from google.rpc import status_pb2

from colossus_sdk.error import (
    ColossusFieldViolation,
    ColossusRetryAfter,
    ColossusRpcError,
    decode_colossus_rpc_error,
)

_TYPE_URL = "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail"


class FakeStatusCode(Enum):
    INVALID_ARGUMENT = (3, "invalid argument")
    INTERNAL = (13, "internal")


class FakeRpcError:
    def __init__(
        self,
        code: FakeStatusCode,
        metadata: Iterable[tuple[str, str | bytes]],
    ) -> None:
        self._code = code
        self._metadata = tuple(metadata)

    def code(self) -> object:
        return self._code

    def trailing_metadata(self) -> Iterable[tuple[str, str | bytes]]:
        return self._metadata


def encoded_status(
    *,
    outcome: int = common_pb2.OUTCOME_CERTAINTY_KNOWN,
    detail_value: bytes | None = None,
    type_url: str = _TYPE_URL,
    detail_count: int = 1,
) -> bytes:
    if detail_value is None:
        detail_value = common_pb2.ColossusErrorDetail(
            reason="INVALID_ARGUMENT",
            request_id="request-123",
            retryable=False,
            retry_after={"seconds": 2, "nanos": 500_000_000},
            outcome_certainty=outcome,
            violations=[
                {
                    "field": "prompt",
                    "description": "must not be empty",
                }
            ],
        ).SerializeToString()
    status = status_pb2.Status(
        code=3,
        message="request rejected",
        details=[
            any_pb2.Any(type_url=type_url, value=detail_value) for _index in range(detail_count)
        ],
    )
    return status.SerializeToString()


class ErrorTests(unittest.TestCase):
    def test_decodes_bounded_canonical_colossus_error(self) -> None:
        error = grpc.aio.AioRpcError(
            grpc.StatusCode.INVALID_ARGUMENT,
            trailing_metadata=grpc.aio.Metadata(("grpc-status-details-bin", encoded_status())),
        )

        self.assertEqual(
            decode_colossus_rpc_error(error),
            ColossusRpcError(
                code=3,
                message="request rejected",
                reason="INVALID_ARGUMENT",
                request_id="request-123",
                retryable=False,
                retry_after=ColossusRetryAfter(
                    seconds=2,
                    nanos=500_000_000,
                ),
                outcome_certainty="known",
                violations=(
                    ColossusFieldViolation(
                        field="prompt",
                        description="must not be empty",
                    ),
                ),
            ),
        )

    def test_rejects_malformed_oversized_duplicate_and_mismatched_details(
        self,
    ) -> None:
        cases = (
            FakeRpcError(
                FakeStatusCode.INVALID_ARGUMENT,
                (("grpc-status-details-bin", b"x" * (64 * 1024 + 1)),),
            ),
            FakeRpcError(
                FakeStatusCode.INVALID_ARGUMENT,
                (("grpc-status-details-bin", encoded_status(detail_count=2)),),
            ),
            FakeRpcError(
                FakeStatusCode.INTERNAL,
                (("grpc-status-details-bin", encoded_status()),),
            ),
            FakeRpcError(
                FakeStatusCode.INVALID_ARGUMENT,
                (
                    (
                        "grpc-status-details-bin",
                        encoded_status(detail_value=b"x" * (16 * 1024 + 1)),
                    ),
                ),
            ),
            FakeRpcError(
                FakeStatusCode.INVALID_ARGUMENT,
                (
                    (
                        "grpc-status-details-bin",
                        encoded_status(outcome=common_pb2.OUTCOME_CERTAINTY_UNSPECIFIED),
                    ),
                ),
            ),
            FakeRpcError(
                FakeStatusCode.INVALID_ARGUMENT,
                (
                    ("grpc-status-details-bin", encoded_status()),
                    ("grpc-status-details-bin", encoded_status()),
                ),
            ),
        )
        for error in cases:
            with self.subTest(error=error):
                self.assertIsNone(decode_colossus_rpc_error(error))

    def test_ignores_non_colossus_detail_without_resolving_type_url(self) -> None:
        error = FakeRpcError(
            FakeStatusCode.INVALID_ARGUMENT,
            (
                (
                    "grpc-status-details-bin",
                    encoded_status(type_url="type.googleapis.com/example.ExternalDetail"),
                ),
            ),
        )
        self.assertIsNone(decode_colossus_rpc_error(error))


if __name__ == "__main__":
    unittest.main()
