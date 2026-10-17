import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readFile, realpath } from "node:fs/promises";
import { join, relative, isAbsolute, dirname } from "node:path";
import { UserError } from "./errors.js";
import type { ManagedConnection } from "./connection.js";

const MAX_FRAME = 2 * 1024 * 1024;
export interface ManagedProfile {
  kind: "managed";
  workspacePath: string;
  workspaceIdentity: string;
  instanceId: string;
  role: string;
}
export interface ManagedModel {
  profile: string;
  provider_profile: string;
  model: string;
  context_window_tokens: number;
  max_output_tokens: number;
  capabilities: {
    tool_calls: "auto";
    streaming: "auto";
    image_inputs: "auto";
    server_compaction: "auto";
    declared: Record<string, never>;
  };
  reasoning_effort?: string;
}
export interface ManagedProvider {
  profile: string;
  kind:
    "echo" | "open_ai_responses" | "open_ai_compatible" | "anthropic_messages";
  base_url: string | null;
  credential_id: string | null;
  timeout_ms: number;
}
export interface ManagedSetup {
  version: 1;
  workspaceIdentity: string;
  nativeWorkspaceIdentity?: { version: number; sha256: string };
  providers: ManagedProvider[];
  models: ManagedModel[];
  primary: string;
  access: "minimal" | "development";
}

export function runtimeConfiguration(setup: ManagedSetup) {
  if (
    setup.version !== 1 ||
    !["minimal", "development"].includes(setup.access) ||
    !Array.isArray(setup.providers) ||
    setup.providers.length < 1 ||
    setup.providers.length > 16 ||
    !Array.isArray(setup.models) ||
    setup.models.length < 1 ||
    setup.models.length > 64 ||
    !setup.models.some((model) => model.profile === setup.primary)
  )
    throw new UserError("Configure a provider and model for Managed Local.");
  return {
    access_profile: setup.access,
    execution_boundary: "full_access",
    providers: setup.providers,
    models: setup.models,
    roles: { primary: setup.primary },
    search_profiles: [],
    search_roles: {},
    mcp_servers: [],
    telemetry: null,
    field_overrides: [],
  };
}

export class PrivateFrames {
  private pending = Buffer.alloc(0);
  constructor(private readonly receive: (value: unknown) => void) {}
  push(chunk: Buffer) {
    this.pending = Buffer.concat([this.pending, chunk]);
    while (this.pending.length >= 4) {
      const size = this.pending.readUInt32BE(0);
      if (!size || size > MAX_FRAME) throw new Error("Invalid private frame");
      if (this.pending.length < size + 4) break;
      const frame = this.pending.subarray(4, size + 4);
      this.pending = this.pending.subarray(size + 4);
      const decoded: unknown = JSON.parse(frame.toString("utf8"));
      frame.fill(0);
      this.receive(decoded);
    }
    if (this.pending.length > MAX_FRAME + 4)
      throw new Error("Invalid private frame");
  }
}

export async function verifyNativeBundle(directory: string) {
  const base = await realpath(directory);
  const manifestFile = join(base, "manifest.json");
  const metadata = await lstat(manifestFile);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 8192)
    throw new UserError(
      "The bundled runtime manifest is invalid. Reinstall the VSIX.",
    );
  const manifest = JSON.parse(await readFile(manifestFile, "utf8")) as Record<
    string,
    unknown
  >;
  if (
    manifest.version !== 1 ||
    manifest.target !== `${process.platform}-${process.arch}`
  )
    throw new UserError(
      "Install the Colossus VSIX for this extension host's platform.",
    );
  const suffix = process.platform === "win32" ? ".exe" : "";
  const verify = async (name: string) => {
    const path = join(base, `${name}${suffix}`);
    const info = await lstat(path);
    const canonical = await realpath(path);
    const child = relative(base, canonical);
    const sha256 = manifest[name];
    if (
      !info.isFile() ||
      info.isSymbolicLink() ||
      info.size > 512 * 1024 * 1024 ||
      !info.size ||
      isAbsolute(child) ||
      child.startsWith("..") ||
      typeof sha256 !== "string" ||
      !/^[a-f0-9]{64}$/u.test(sha256)
    )
      throw new UserError(
        "The bundled runtime is invalid. Reinstall the VSIX.",
      );
    const digest = createHash("sha256");
    for await (const chunk of createReadStream(path)) digest.update(chunk);
    if (digest.digest("hex") !== sha256)
      throw new UserError(
        "The bundled runtime failed verification. Reinstall the VSIX.",
      );
    return { path, sha256 };
  };
  return {
    host: await verify("colossus-app-host"),
    sidecar: await verify("colossus-sidecar"),
    macosDeveloperPreview: manifest.macos_developer_preview === true,
  };
}

/** Parent-pipe lifetime: disposing the sidebar does not stop this owner. */
export class ManagedRuntime {
  private generation = 0;
  private child: ChildProcessWithoutNullStreams | undefined;
  private stopping: Promise<void> | undefined;
  private current: ManagedConnection | undefined;
  private binding: ManagedSetup["nativeWorkspaceIdentity"];
  get workspaceIdentity() {
    return this.binding;
  }
  get alive() {
    return !!this.child && this.child.exitCode === null && !this.stopping;
  }
  get connection() {
    return this.current;
  }
  get pid() {
    return this.child?.pid ?? 0;
  }

  async start(options: {
    bundle: string;
    workspace: string;
    home: string;
    setup: ManagedSetup;
    credentials: { id: string; secret: string }[];
    stopped: () => void;
    restarted: () => void;
  }): Promise<ManagedConnection> {
    const generation = ++this.generation;
    await this.closeOwned();
    const runtime = runtimeConfiguration(options.setup);
    const bundle = await verifyNativeBundle(options.bundle);
    if (generation !== this.generation)
      throw new UserError("Managed Local startup was cancelled.");
    const payload = Buffer.from(
      JSON.stringify({
        version: 1,
        workspace: options.workspace,
        home: options.home,
        executable: bundle.sidecar.path,
        sha256: bundle.sidecar.sha256,
        macos_developer_preview: bundle.macosDeveloperPreview,
        expected_workspace_identity:
          options.setup.nativeWorkspaceIdentity ?? null,
        runtime,
        credentials: options.credentials,
      }),
    );
    if (payload.length > MAX_FRAME) {
      payload.fill(0);
      throw new UserError("Managed configuration is too large.");
    }
    const environment: NodeJS.ProcessEnv = {};
    for (const key of [
      "SystemRoot",
      "WINDIR",
      "HOME",
      "USERPROFILE",
      "LOCALAPPDATA",
      "DBUS_SESSION_BUS_ADDRESS",
      "XDG_RUNTIME_DIR",
    ])
      if (process.env[key]) environment[key] = process.env[key];
    const child = spawn(bundle.host.path, ["--managed-v1"], {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
      cwd: dirname(bundle.host.path),
      env: environment,
    });
    this.child = child;
    child.stderr.resume(); // Never forward native diagnostics containing private data.
    const connection = new Promise<ManagedConnection>((resolve, reject) => {
      const timeout = setTimeout(() => fail(), 60_000);
      const fail = (code?: unknown) => {
        clearTimeout(timeout);
        if (child === this.child) {
          this.current = undefined;
          this.binding = undefined;
        }
        const detail =
          code === "workspace_busy"
            ? "Another Colossus owner is using this workspace's managed state. Close that owner before starting here."
            : code === "workspace_changed"
              ? "The workspace changed during startup. Select the original folder again."
              : code === "bundle_verification_failed"
                ? "The bundled runtime failed native verification. Reinstall the VSIX."
                : code === "invalid_configuration"
                  ? "The managed provider or model configuration was rejected. Use Configure Local Runtime to correct it."
                  : "Managed Local could not start. Check the provider setup and native credential store, then retry.";
        reject(new UserError(detail));
        child.stdin.destroy();
      };
      const frames = new PrivateFrames((value) => {
        const message = value as {
          type?: string;
          connection?: ManagedConnection;
          code?: string;
          workspace_identity?: { version: number; sha256: string };
        };
        if (
          message?.type === "connection" &&
          message.connection &&
          child === this.child &&
          !this.stopping
        ) {
          const identity = message.workspace_identity;
          if (
            !identity ||
            ![2, 3, 4, 5].includes(identity.version) ||
            !/^[a-f0-9]{64}$/u.test(identity.sha256)
          ) {
            fail();
            return;
          }
          const previous = this.current;
          if (
            previous &&
            (previous.instance_id !== message.connection.instance_id ||
              this.binding?.version !== identity.version ||
              this.binding?.sha256 !== identity.sha256)
          ) {
            fail();
            return;
          }
          this.binding = identity;
          this.current = message.connection;
          clearTimeout(timeout);
          resolve(message.connection);
          if (previous) options.restarted();
        } else if (message?.type === "failed") fail(message.code);
      });
      child.stdout.on("data", (chunk: Buffer) => {
        try {
          frames.push(chunk);
        } catch {
          fail();
        }
      });
      child.once("error", fail);
      child.once("close", () => {
        clearTimeout(timeout);
        reject(new UserError("Managed Local stopped before connecting."));
        if (this.child === child) {
          this.child = undefined;
          this.current = undefined;
          this.binding = undefined;
          options.stopped();
        }
      });
    });
    const header = Buffer.alloc(4);
    header.writeUInt32BE(payload.length);
    child.stdin.on("error", () => {});
    child.stdin.write(header);
    child.stdin.write(payload, () => payload.fill(0));
    try {
      return await connection;
    } catch (error) {
      if (child === this.child) await this.stop();
      throw error;
    }
  }

  stop(): Promise<void> {
    this.generation++;
    return this.closeOwned();
  }

  private closeOwned(): Promise<void> {
    if (this.stopping) return this.stopping;
    const child = this.child;
    this.current = undefined;
    this.binding = undefined;
    if (!child || child.exitCode !== null || child.signalCode !== null)
      return Promise.resolve();
    this.stopping = new Promise<void>((resolve) => {
      const timer = setTimeout(() => child.kill(), 45_000);
      child.once("close", () => {
        clearTimeout(timer);
        resolve();
      });
      child.stdin.end(); // SDK drains, checkpoints and closes the owned process tree.
    }).finally(() => {
      if (this.child === child) this.child = undefined;
      this.stopping = undefined;
    });
    return this.stopping;
  }
}
