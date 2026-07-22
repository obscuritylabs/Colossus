import assert from "node:assert/strict";
import { test } from "node:test";

import * as grpc from "@grpc/grpc-js";

import {
  ColossusErrorDetail,
  OutcomeCertainty,
} from "../src/gen/colossus/api/v1alpha1/common.js";
import { Status } from "../src/gen/google/rpc/status.js";
import { decodeColossusRpcError } from "../src/error.js";

const TYPE_URL =
  "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail";

function serviceError(
  statusBytes: Uint8Array,
  code: grpc.status = grpc.status.INVALID_ARGUMENT,
): Pick<grpc.ServiceError, "code" | "metadata"> {
  const metadata = new grpc.Metadata();
  metadata.set("grpc-status-details-bin", Buffer.from(statusBytes));
  return { code, metadata };
}

function encodedStatus(options?: {
  detailBytes?: Uint8Array;
  detailTypeUrl?: string;
  details?: number;
  outcome?: OutcomeCertainty;
}): Uint8Array {
  const detail =
    options?.detailBytes ??
    ColossusErrorDetail.encode({
      reason: "INVALID_ARGUMENT",
      requestId: "request-123",
      retryable: false,
      retryAfter: { seconds: 2n, nanos: 500_000_000 },
      outcomeCertainty:
        options?.outcome ?? OutcomeCertainty.OUTCOME_CERTAINTY_KNOWN,
      violations: [{ field: "prompt", description: "must not be empty" }],
    }).finish();
  const details = Array.from({ length: options?.details ?? 1 }, () => ({
    typeUrl: options?.detailTypeUrl ?? TYPE_URL,
    value: Buffer.from(detail),
  }));
  return Status.encode({
    code: grpc.status.INVALID_ARGUMENT,
    message: "request rejected",
    details,
  }).finish();
}

test("decodes a bounded canonical Colossus rich error", () => {
  const decoded = decodeColossusRpcError(serviceError(encodedStatus()));
  assert.deepEqual(decoded, {
    code: grpc.status.INVALID_ARGUMENT,
    message: "request rejected",
    reason: "INVALID_ARGUMENT",
    requestId: "request-123",
    retryable: false,
    retryAfter: { seconds: 2n, nanos: 500_000_000 },
    outcomeCertainty: "known",
    violations: [{ field: "prompt", description: "must not be empty" }],
  });
  assert.equal(Object.isFrozen(decoded), true);
  assert.equal(Object.isFrozen(decoded?.violations), true);
});

test("rejects malformed, oversized, duplicated, and mismatched status details", () => {
  assert.equal(
    decodeColossusRpcError(
      serviceError(Buffer.alloc(64 * 1024 + 1)),
    ),
    undefined,
  );
  assert.equal(
    decodeColossusRpcError(serviceError(encodedStatus({ details: 2 }))),
    undefined,
  );
  assert.equal(
    decodeColossusRpcError(
      serviceError(encodedStatus(), grpc.status.INTERNAL),
    ),
    undefined,
  );
  assert.equal(
    decodeColossusRpcError(
      serviceError(
        encodedStatus({
          detailBytes: Buffer.alloc(16 * 1024 + 1),
        }),
      ),
    ),
    undefined,
  );
  assert.equal(
    decodeColossusRpcError(
      serviceError(
        encodedStatus({
          outcome: OutcomeCertainty.OUTCOME_CERTAINTY_UNSPECIFIED,
        }),
      ),
    ),
    undefined,
  );
});

test("ignores non-Colossus details without resolving type URLs", () => {
  assert.equal(
    decodeColossusRpcError(
      serviceError(
        encodedStatus({
          detailTypeUrl: "type.googleapis.com/example.ExternalDetail",
        }),
      ),
    ),
    undefined,
  );
});
