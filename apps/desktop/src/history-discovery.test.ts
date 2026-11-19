import { describe, expect, it } from "vitest";
import {
  historyOperationProof,
  historyDiscoveryRequest,
} from "./history-discovery";
import type { Run } from "./types";

function run(
  runId: string,
  sessionId: string,
  discoveryProof?: string,
  controllable = false,
): Run {
  return {
    runId,
    sessionId,
    title: "History",
    role: "primary",
    mode: "execute",
    status: "completed",
    createdAt: "2026-11-18T00:00:00Z",
    updatedAt: "2026-11-18T00:00:00Z",
    startedAt: null,
    finishedAt: null,
    lastSequence: 1,
    pendingInteractionCount: 0,
    terminal: null,
    etag: "etag",
    archived: false,
    controllable,
    ...(discoveryProof ? { discoveryProof } : {}),
  };
}

describe("historyDiscoveryRequest", () => {
  it("forwards the selected native receipt and a candidate from the same session", () => {
    const rows = [
      run("other", "different", "other-proof", true),
      run("cloud", "shared", "opaque-cloud"),
      run("primary", "shared", "opaque-primary", true),
    ];
    expect(historyDiscoveryRequest({ runId: "cloud" }, rows)).toEqual({
      runId: "cloud",
      discoveryProof: "opaque-cloud",
      continuationProof: "opaque-primary",
    });
  });

  it("does not forward another session or infer a primary proof from an unavailable flag", () => {
    expect(
      historyDiscoveryRequest({ runId: "selected" }, [
        run("selected", "one"),
        run("other", "two", "other-proof", true),
        run("cloud", "one", "cloud-proof"),
      ]),
    ).toEqual({ runId: "selected" });
  });

  it("preserves explicit opaque receipts without parsing them", () => {
    const request = {
      runId: "selected",
      discoveryProof: "opaque-own",
      continuationProof: "opaque-continuation",
    };
    expect(historyDiscoveryRequest(request, [])).toEqual(request);
  });

  it("forwards only an operation's exact run receipt without a continuation receipt", () => {
    const rows = [
      run("other", "shared", "other-proof", true),
      run("selected", "shared", "selected-proof"),
    ];
    expect(historyOperationProof("selected", rows)).toEqual({
      discoveryProof: "selected-proof",
    });
    expect(historyOperationProof("missing", rows)).toEqual({});
  });
});
