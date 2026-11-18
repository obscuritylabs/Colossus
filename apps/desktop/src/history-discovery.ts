import type { GetRunRequest, Run } from "./types";

/** Forward native receipts unchanged. Only the native reader can validate them. */
export function historyDiscoveryRequest(
  request: GetRunRequest,
  runs: readonly Run[],
): GetRunRequest {
  const selected = runs.find((run) => run.runId === request.runId);
  const discoveryProof =
    request.discoveryProof ??
    runs.find((run) => run.runId === request.runId && run.discoveryProof)
      ?.discoveryProof;
  const continuationProof =
    request.continuationProof ??
    (selected
      ? runs.find(
          (run) =>
            run.sessionId === selected.sessionId &&
            run.controllable === true &&
            run.discoveryProof,
        )?.discoveryProof
      : undefined);
  return {
    ...request,
    ...(discoveryProof ? { discoveryProof } : {}),
    ...(continuationProof ? { continuationProof } : {}),
  };
}

/** Controls and inspection carry only their exact run's opaque receipt. */
export function historyOperationProof(
  runId: string,
  runs: readonly Run[],
): { discoveryProof?: string } {
  const discoveryProof = runs.find(
    (run) => run.runId === runId && run.discoveryProof,
  )?.discoveryProof;
  return discoveryProof ? { discoveryProof } : {};
}
