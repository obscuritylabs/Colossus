import { constants } from "node:fs";
import { open, realpath, stat } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import * as grpc from "@grpc/grpc-js";
import {
  StaticBearerCredential,
  assertPinnedLeafCertificate,
  createSecureGrpcClient,
  parseEndpointDescriptor,
  decodeColossusRpcError,
  isTerminalRunUpdate,
  watchRun,
} from "@obscuritylabs/colossus-sdk";
import {
  AgentRunServiceClient,
  type CreateRunRequest,
  type CreateRunResponse,
  type ListRunsResponse,
  type ListSessionActivityResponse,
  type CancelRunResponse,
  type RespondInteractionResponse,
  type RunUpdate,
  type WatchRunResponse,
  type RespondInteractionRequest,
  type GetRunResponse,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";

import {
  DeploymentMode,
  SystemServiceClient,
  type ServerInfo,
  type GetServerInfoResponse,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/system";
import {
  ConnectionError,
  UserError,
  type ConnectionDiagnostic,
  type ConnectionStage,
} from "./errors.js";

function connectionFailure(
  stage: ConnectionStage,
  error: unknown,
): ConnectionError {
  if (error instanceof ConnectionError) return error;
  if (error instanceof UserError)
    return new ConnectionError(stage, "validation", error.message);
  const code =
    error && typeof error === "object" && "code" in error
      ? error.code
      : undefined;
  if (code === "ENOENT")
    return new ConnectionError(
      stage,
      "missing-file",
      "The selected folder or discovery file is missing. Keep the worker running with --public-api-dir and select the directory it publishes.",
    );
  if (code === "EACCES" || code === "EPERM")
    return new ConnectionError(
      stage,
      "file-access",
      "VS Code cannot access the selected folder or discovery file. Check its owner and permissions.",
    );
  if (code === "ELOOP")
    return new ConnectionError(
      stage,
      "symlink",
      "A discovery file is a symbolic link. Select the worker's original discovery directory.",
    );
  if (code === grpc.status.UNAUTHENTICATED)
    return new ConnectionError(
      stage,
      "unauthenticated",
      "The worker rejected the enrolled credential. Check the saved connection against the original enrollment's worker and keyring service/account.",
    );
  if (code === grpc.status.PERMISSION_DENIED)
    return new ConnectionError(
      stage,
      "permission-denied",
      "The worker denied this read. Check the application's enrollment scopes and role.",
    );
  if (code === grpc.status.UNAVAILABLE)
    return new ConnectionError(
      stage,
      "unavailable",
      "The worker is unavailable or its TLS handshake failed. Keep the worker running and check the discovery directory and enrollment anchors.",
    );
  if (code === grpc.status.DEADLINE_EXCEEDED)
    return new ConnectionError(
      stage,
      "timeout",
      "The worker did not respond in time. Check that the worker is still running.",
    );
  const messages: Record<ConnectionStage, string> = {
    workspace:
      "The workspace could not be verified. Select the original workspace folder.",
    "discovery-directory":
      "The discovery directory could not be verified. Select the directory published by the worker.",
    "endpoint-file":
      "endpoint.json could not be read. Check the discovery directory and file permissions.",
    "endpoint-descriptor":
      "endpoint.json is invalid or unsupported. Use the discovery directory published by a compatible Colossus CLI.",
    "certificate-file":
      "certificate.pem could not be read. Check the discovery directory and file permissions.",
    "certificate-pin":
      "The worker certificate or instance identity does not match the saved enrollment, or the certificate is invalid. Compare with the original enrollment output; use Forget Connection to re-enter those values.",
    keyring:
      "The OS-keyring credential could not be loaded. Check the native binding, unlock the credential store, and verify the enrollment's service/account.",
    handshake:
      "The authenticated worker handshake failed. Check the saved enrollment, CLI/extension versions, and that the worker is running.",
    "server-info":
      "Authenticated worker information could not be read. Check the worker connection.",
    "saved-profile":
      "The protected connection profile could not be read or saved. Check VS Code SecretStorage; use Forget Connection to configure it again.",
    history:
      "Connection was established, but run history could not be loaded. Check the worker and runs:read scope.",
  };
  return new ConnectionError(stage, "failed", messages[stage]);
}

/** Reports only fixed stage/category values. Never logs raw errors, metadata, paths, or credentials. */
export async function connectionStep<T>(
  stage: ConnectionStage,
  operation: () => PromiseLike<T>,
  report: (event: ConnectionDiagnostic) => void = () => {},
): Promise<T> {
  report({ stage, state: "started" });
  try {
    const value = await operation();
    report({ stage, state: "succeeded" });
    return value;
  } catch (error) {
    const failure = connectionFailure(stage, error);
    report({ stage, state: "failed", reason: failure.reason });
    throw failure;
  }
}

export interface ConnectionProfile {
  schemaVersion: 1;
  workspacePath: string;
  workspaceIdentity: string;
  discoveryDirectory: string;
  instanceId: string;
  certificateSha256: string;
  keyringService: string;
  keyringAccount: string;
  role: string;
}

export function parseProfile(value: unknown): ConnectionProfile {
  if (!value || typeof value !== "object")
    throw new UserError("Invalid connection profile.");
  const p = value as Record<string, unknown>;
  const keys = [
    "schemaVersion",
    "workspacePath",
    "workspaceIdentity",
    "discoveryDirectory",
    "instanceId",
    "certificateSha256",
    "keyringService",
    "keyringAccount",
    "role",
  ];
  if (
    Object.keys(p).length !== keys.length ||
    keys.some((k) => !(k in p)) ||
    p.schemaVersion !== 1 ||
    keys
      .slice(1)
      .some((k) => typeof p[k] !== "string" || (p[k] as string).length > 4096)
  )
    throw new UserError("Invalid connection profile.");
  if (
    !isAbsolute(p.workspacePath as string) ||
    !isAbsolute(p.discoveryDirectory as string) ||
    !/^[0-9a-f-]{36}$/u.test(p.instanceId as string) ||
    !/^[0-9a-f]{64}$/u.test(p.certificateSha256 as string) ||
    !/^[A-Za-z0-9._:-]{1,128}$/u.test(p.keyringService as string) ||
    !/^[A-Za-z0-9._:-]{1,128}$/u.test(p.keyringAccount as string) ||
    !/^[A-Za-z0-9._-]{1,64}$/u.test(p.role as string)
  )
    throw new UserError("Invalid connection enrollment values.");
  return p as unknown as ConnectionProfile;
}

export async function workspaceIdentity(
  path: string,
): Promise<{ path: string; identity: string }> {
  const canonical = await realpath(path);
  const info = await stat(canonical, { bigint: true });
  if (!info.isDirectory()) throw new UserError("Select a workspace folder.");
  return {
    path: canonical,
    identity: `${info.dev}:${info.ino}:${info.birthtimeNs}`,
  };
}

async function boundedPrivateFile(
  path: string,
  maximum: number,
): Promise<Buffer> {
  const handle = await open(
    path,
    constants.O_RDONLY |
      (process.platform === "win32" ? 0 : constants.O_NOFOLLOW),
  );
  try {
    const info = await handle.stat();
    if (
      !info.isFile() ||
      info.size > maximum ||
      (process.platform !== "win32" &&
        ((info.mode & 0o077) !== 0 || info.uid !== process.getuid?.()))
    )
      throw new UserError(
        "Worker discovery files must be bounded and owner-private.",
      );
    const bytes = Buffer.alloc(maximum + 1);
    let offset = 0;
    while (offset < bytes.length) {
      const { bytesRead } = await handle.read(
        bytes,
        offset,
        bytes.length - offset,
        null,
      );
      if (bytesRead === 0) break;
      offset += bytesRead;
    }
    if (offset > maximum)
      throw new UserError("Worker discovery file exceeds its size limit.");
    return bytes.subarray(0, offset);
  } finally {
    await handle.close();
  }
}

export async function connectWorker(
  profile: ConnectionProfile,
  readCredential: (service: string, account: string) => Promise<string | null>,
  report: (event: ConnectionDiagnostic) => void = () => {},
): Promise<WorkerClient> {
  const p = parseProfile(profile);
  const workspace = await connectionStep(
    "workspace",
    () => workspaceIdentity(p.workspacePath),
    report,
  );
  if (
    workspace.path !== p.workspacePath ||
    workspace.identity !== p.workspaceIdentity
  )
    throw new UserError("The workspace has changed. Enroll this folder again.");
  const directory = await connectionStep(
    "discovery-directory",
    async () => {
      const directory = await realpath(p.discoveryDirectory);
      if (directory !== p.discoveryDirectory)
        throw new UserError("Worker discovery directory must be canonical.");
      const info = await stat(directory);
      if (
        !info.isDirectory() ||
        (process.platform !== "win32" &&
          ((info.mode & 0o077) !== 0 || info.uid !== process.getuid?.()))
      )
        throw new UserError(
          "Worker discovery directory must be owner-private.",
        );
      return directory;
    },
    report,
  );
  const endpoint = await connectionStep(
    "endpoint-file",
    () => boundedPrivateFile(join(directory, "endpoint.json"), 32 * 1024),
    report,
  );
  const descriptor = await connectionStep(
    "endpoint-descriptor",
    async () => parseEndpointDescriptor(endpoint.toString("utf8")),
    report,
  );
  const certificate = await connectionStep(
    "certificate-file",
    () => boundedPrivateFile(join(directory, "certificate.pem"), 64 * 1024),
    report,
  );
  // Discovery is mutable: verify independent protected enrollment anchors before keyring access.
  await connectionStep(
    "certificate-pin",
    async () =>
      assertPinnedLeafCertificate(
        descriptor,
        certificate,
        p.instanceId,
        p.certificateSha256,
      ),
    report,
  );
  const credential = await connectionStep(
    "keyring",
    async () => {
      const token = await readCredential(p.keyringService, p.keyringAccount);
      if (token === null)
        throw new ConnectionError(
          "keyring",
          "credential-not-found",
          "No enrolled credential was found at the saved OS-keyring service/account. Use Credential Location to compare those names with enrollment, then reconnect.",
        );
      return new StaticBearerCredential(token);
    },
    report,
  );
  const runs = await connectionStep(
    "handshake",
    () =>
      createSecureGrpcClient(
        AgentRunServiceClient,
        descriptor,
        certificate,
        p.instanceId,
        p.certificateSha256,
        DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
        credential,
      ),
    report,
  );
  const channel = runs.getChannel();
  const options = { channelOverride: channel };
  const system = new SystemServiceClient(
    descriptor.target,
    grpc.credentials.createInsecure(),
    options,
  );
  try {
    const metadata = await connectionStep(
      "server-info",
      () =>
        rpc<GetServerInfoResponse>((callback) =>
          system.getServerInfo(
            {},
            new grpc.Metadata(),
            { deadline: Date.now() + 5000 },
            callback,
          ),
        ),
      report,
    );
    if (!metadata.serverInfo)
      throw new UserError("Missing authenticated server information.");
    return new WorkerClient(runs, system, metadata.serverInfo);
  } catch (error) {
    runs.close();
    throw error;
  }
}

export function rpc<T>(
  invoke: (
    callback: (error: grpc.ServiceError | null, response: T) => void,
  ) => unknown,
): Promise<T> {
  return new Promise((resolve, reject) =>
    invoke((error, response) => (error ? reject(error) : resolve(response))),
  );
}

export function safeError(error: unknown): string {
  if (error instanceof UserError) return error.message;
  if (
    error &&
    typeof error === "object" &&
    "code" in error &&
    "metadata" in error
  ) {
    try {
      const detail = decodeColossusRpcError(error as grpc.ServiceError);
      if (detail)
        return `Colossus: ${detail.reason}. Outcome: ${detail.outcomeCertainty}. Reconcile the run before retrying an action.`;
    } catch {
      /* Raw transport and keyring errors may contain private data. */
    }
  }
  return "The operation could not be completed. Check the worker, enrollment, and workspace, then reconnect. Actions are never replayed automatically.";
}

export class WorkerClient {
  private listTail: Promise<void> = Promise.resolve();
  private closed = false;

  constructor(
    readonly runs: AgentRunServiceClient,
    private readonly system: SystemServiceClient,
    readonly info: ServerInfo,
  ) {}
  create(request: CreateRunRequest) {
    return rpc<CreateRunResponse>((cb) =>
      this.runs.createRun(
        request,
        new grpc.Metadata(),
        { deadline: Date.now() + 30_000 },
        cb,
      ),
    );
  }
  get(runId: string) {
    return rpc<GetRunResponse>((cb) =>
      this.runs.getRun(
        { runId },
        new grpc.Metadata(),
        { deadline: Date.now() + 30_000 },
        cb,
      ),
    );
  }
  listRuns(sessionId = "", pageToken = "") {
    // Public list admission permits one concurrent read per application and a small
    // burst. Serialize reads and use Desktop's bounded admission-refill schedule.
    // CreateRun, cancellation, and interaction responses are never retried here.
    const operation = this.listTail.then(async () => {
      for (const delay of [100, 400, 500, undefined]) {
        if (this.closed) throw new UserError("Worker connection is closed.");
        try {
          return await rpc<ListRunsResponse>((cb) =>
            this.runs.listRuns(
              {
                sessionId: sessionId || undefined,
                statuses: [],
                includeArchived: false,
                page: { pageSize: 50, pageToken },
              },
              new grpc.Metadata(),
              { deadline: Date.now() + 30_000 },
              cb,
            ),
          );
        } catch (error) {
          const detail =
            error &&
            typeof error === "object" &&
            "code" in error &&
            "metadata" in error
              ? decodeColossusRpcError(error as grpc.ServiceError)
              : undefined;
          if (
            delay === undefined ||
            detail?.code !== grpc.status.RESOURCE_EXHAUSTED ||
            detail.reason !== "capacity_exceeded" ||
            !detail.retryable
          )
            throw error;
          await new Promise((resolve) => setTimeout(resolve, delay));
        }
      }
      throw new UserError("Run history could not be read.");
    });
    this.listTail = operation.then(
      () => {},
      () => {},
    );
    return operation;
  }
  activity(sourceRunId: string) {
    return rpc<ListSessionActivityResponse>((cb) =>
      this.runs.listSessionActivity(
        {
          sourceRunId,
          query: "",
          lanes: [],
          kinds: [],
          statuses: [],
          page: { pageSize: 25, pageToken: "" },
        },
        new grpc.Metadata(),
        { deadline: Date.now() + 30_000 },
        cb,
      ),
    );
  }
  cancel(runId: string, idempotencyKey: string) {
    return rpc<CancelRunResponse>((cb) =>
      this.runs.cancelRun(
        { runId, idempotencyKey },
        new grpc.Metadata(),
        { deadline: Date.now() + 30_000 },
        cb,
      ),
    );
  }
  respond(request: RespondInteractionRequest) {
    return rpc<RespondInteractionResponse>((cb) =>
      this.runs.respondInteraction(
        request,
        new grpc.Metadata(),
        { deadline: Date.now() + 30_000 },
        cb,
      ),
    );
  }
  watch(runId: string, signal: AbortSignal) {
    const client = this;
    return watchRun<RunUpdate>({
      runId,
      signal,
      isTerminal: isTerminalRunUpdate,
      open: async function* (id, afterSequence, abort) {
        const stream = client.runs.watchRun({ runId: id, afterSequence });
        const cancel = () => stream.cancel();
        abort?.addEventListener("abort", cancel, { once: true });
        if (abort?.aborted) stream.cancel();
        try {
          for await (const response of stream as AsyncIterable<WatchRunResponse>) {
            if (!response.update) throw new UserError("Empty run feed item.");
            yield {
              runId: response.update.runId,
              sequence: response.update.sequence,
              value: response.update,
            };
          }
        } finally {
          abort?.removeEventListener("abort", cancel);
          stream.cancel();
        }
      },
      reconcile: async (id) => {
        const response: GetRunResponse = await client.get(id);
        if (!response.run)
          throw new UserError("Missing run during reconciliation.");
        return {
          runId: id,
          lastSequence: response.run.lastSequence,
          terminal: response.run.terminal !== undefined,
        };
      },
    });
  }
  close() {
    this.closed = true;
    this.system.close();
    this.runs.close();
  }
}
