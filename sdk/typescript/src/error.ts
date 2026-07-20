import type * as grpc from "@grpc/grpc-js";

import {
  ColossusErrorDetail as GeneratedColossusErrorDetail,
  OutcomeCertainty as GeneratedOutcomeCertainty,
} from "./gen/colossus/api/v1alpha1/common.js";
import { Status as GeneratedStatus } from "./gen/google/rpc/status.js";

const STATUS_METADATA_KEY = "grpc-status-details-bin";
const COLOSSUS_ERROR_TYPE_URL =
  "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail";
const MAX_STATUS_BYTES = 64 * 1024;
const MAX_STATUS_MESSAGE_BYTES = 1024;
const MAX_DETAILS = 16;
const MAX_DETAIL_BYTES = 16 * 1024;
const MAX_REASON_BYTES = 128;
const MAX_REQUEST_ID_BYTES = 128;
const MAX_VIOLATIONS = 32;
const MAX_VIOLATION_FIELD_BYTES = 256;
const MAX_VIOLATION_DESCRIPTION_BYTES = 1024;

export type ErrorOutcomeCertainty = "known" | "unknown";

export interface ColossusFieldViolation {
  readonly field: string;
  readonly description: string;
}

export interface ColossusRetryAfter {
  readonly seconds: bigint;
  readonly nanos: number;
}

/**
 * A bounded, transport-independent view of a Colossus rich gRPC error.
 *
 * `retryable` is informational. The SDK never automatically retries effectful
 * calls, and callers must not replay an effect whose outcome is `unknown`
 * without first reconciling its durable state.
 */
export interface ColossusRpcError {
  readonly code: number;
  readonly message: string;
  readonly reason: string;
  readonly requestId: string;
  readonly retryable: boolean;
  readonly retryAfter?: ColossusRetryAfter;
  readonly outcomeCertainty: ErrorOutcomeCertainty;
  readonly violations: readonly ColossusFieldViolation[];
}

function utf8Length(value: string): number {
  return Buffer.byteLength(value, "utf8");
}

function hasBoundedLength(value: string, maximum: number): boolean {
  return utf8Length(value) <= maximum;
}

function normalizeOutcome(
  outcome: GeneratedOutcomeCertainty,
): ErrorOutcomeCertainty | undefined {
  switch (outcome) {
    case GeneratedOutcomeCertainty.OUTCOME_CERTAINTY_KNOWN:
      return "known";
    case GeneratedOutcomeCertainty.OUTCOME_CERTAINTY_UNKNOWN:
      return "unknown";
    default:
      return undefined;
  }
}

function decodeStatusBytes(
  rawStatus: Buffer,
  transportCode: number,
): ColossusRpcError | undefined {
  if (rawStatus.length === 0 || rawStatus.length > MAX_STATUS_BYTES) {
    return undefined;
  }

  try {
    const status = GeneratedStatus.decode(rawStatus);
    if (
      !Number.isInteger(status.code) ||
      status.code < 1 ||
      status.code > 16 ||
      status.code !== transportCode ||
      !hasBoundedLength(status.message, MAX_STATUS_MESSAGE_BYTES) ||
      status.details.length > MAX_DETAILS
    ) {
      return undefined;
    }

    const matchingDetails = status.details.filter(
      (detail) => detail.typeUrl === COLOSSUS_ERROR_TYPE_URL,
    );
    if (matchingDetails.length !== 1) {
      return undefined;
    }
    const packedDetail = matchingDetails[0];
    if (
      packedDetail === undefined ||
      packedDetail.value.length > MAX_DETAIL_BYTES
    ) {
      return undefined;
    }

    const detail = GeneratedColossusErrorDetail.decode(packedDetail.value);
    const outcomeCertainty = normalizeOutcome(detail.outcomeCertainty);
    if (
      outcomeCertainty === undefined ||
      !hasBoundedLength(detail.reason, MAX_REASON_BYTES) ||
      !hasBoundedLength(detail.requestId, MAX_REQUEST_ID_BYTES) ||
      detail.violations.length > MAX_VIOLATIONS
    ) {
      return undefined;
    }

    const violations: ColossusFieldViolation[] = [];
    for (const violation of detail.violations) {
      if (
        !hasBoundedLength(violation.field, MAX_VIOLATION_FIELD_BYTES) ||
        !hasBoundedLength(
          violation.description,
          MAX_VIOLATION_DESCRIPTION_BYTES,
        )
      ) {
        return undefined;
      }
      violations.push(
        Object.freeze({
          field: violation.field,
          description: violation.description,
        }),
      );
    }

    let retryAfter: ColossusRetryAfter | undefined;
    if (detail.retryAfter !== undefined) {
      const { seconds, nanos } = detail.retryAfter;
      if (
        seconds < 0n ||
        seconds > 315_576_000_000n ||
        !Number.isInteger(nanos) ||
        nanos < 0 ||
        nanos > 999_999_999
      ) {
        return undefined;
      }
      retryAfter = Object.freeze({ seconds, nanos });
    }

    return Object.freeze({
      code: status.code,
      message: status.message,
      reason: detail.reason,
      requestId: detail.requestId,
      retryable: detail.retryable,
      ...(retryAfter === undefined ? {} : { retryAfter }),
      outcomeCertainty,
      violations: Object.freeze(violations),
    });
  } catch {
    return undefined;
  }
}

/**
 * Decodes one canonical `grpc-status-details-bin` trailer from a gRPC error.
 *
 * Malformed, oversized, duplicated, non-Colossus, or transport-code-mismatched
 * details return `undefined`; this helper never performs a retry or logs content.
 */
export function decodeColossusRpcError(
  error: Pick<grpc.ServiceError, "code" | "metadata">,
): ColossusRpcError | undefined {
  if (
    !Number.isInteger(error.code) ||
    error.code < 1 ||
    error.code > 16
  ) {
    return undefined;
  }

  try {
    const values = error.metadata.get(STATUS_METADATA_KEY);
    if (
      values.length !== 1 ||
      !Buffer.isBuffer(values[0])
    ) {
      return undefined;
    }
    return decodeStatusBytes(values[0], error.code);
  } catch {
    return undefined;
  }
}
