import { invoke, isTauri } from "@tauri-apps/api/core";
import type { Membership } from "@colossus/control-plane-client";

export interface Identity {
  user: { id: string; display_name: string; is_admin: boolean };
  projects: { id: string; name: string; archived: boolean }[];
  memberships: Membership[];
}
export interface AuthConfiguration {
  local_enabled: boolean;
  oidc: { label: string; login_url: string } | null;
  classification?: { enabled: boolean; text: string; tone: string };
}
export type Operation =
  | { kind: "me" }
  | {
      kind: "fleet" | "hosts" | "threads" | "tasks";
      project: string;
      after?: string;
    }
  | { kind: "thread"; project: string; thread: string }
  | { kind: "task"; project: string; task: string }
  | { kind: "task_updates"; project: string; task: string; after: number }
  | {
      kind: "create_thread";
      project: string;
      node: string;
      request: RunRequest;
    }
  | {
      kind: "send_message";
      project: string;
      thread: string;
      revision: number;
      request: RunRequest;
    }
  | { kind: "cancel"; project: string; task: string; mutation_id: string };
export interface RunRequest {
  input: { text: string }[];
  role: string;
  mode: "execute" | "plan";
  max_turns: number;
  idempotency_key: string;
  plugin_skill_ids: string[];
  session_id: null;
  end_user_id: null;
  research_depth: null;
  research_sources: [];
  plan_action: null;
  branch: null;
}
export interface MobileClient {
  readonly preview: boolean;
  connect(origin: string): Promise<AuthConfiguration>;
  login(username: string, password: string): Promise<Identity>;
  request<T>(operation: Operation): Promise<T>;
  logout(): Promise<void>;
  openBrowser(project?: string, task?: string): Promise<void>;
}
export class ClientFailure extends Error {
  constructor(
    public readonly code: string,
    message: string,
  ) {
    super(message);
  }
}
export function normalizeOrigin(value: string, development = false): string {
  let url: URL;
  try {
    url = new URL(value.trim());
  } catch {
    throw new ClientFailure(
      "invalid_origin",
      "Enter a complete HTTPS server address.",
    );
  }
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
  if (
    !(
      url.protocol === "https:" ||
      (development && local && url.protocol === "http:")
    ) ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  ) {
    throw new ClientFailure(
      "invalid_origin",
      "Use the HTTPS server address without a path, username, or password.",
    );
  }
  return url.origin;
}
async function native<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauri())
    throw new ClientFailure(
      "native_required",
      "Open the installed Colossus Mobile app to connect to your server.",
    );
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const failure = error as { code?: string; message?: string };
    if (failure.code === "session_expired")
      window.dispatchEvent(new Event("colossus:mobile-session-expired"));
    throw new ClientFailure(
      failure.code ?? "unavailable",
      failure.message ?? "The request could not be completed.",
    );
  }
}
export const nativeClient: MobileClient = {
  preview: false,
  connect: (origin) => native("mobile_connect", { origin }),
  login: (username, password) => native("mobile_login", { username, password }),
  request: (operation) => native("mobile_request", { operation }),
  logout: () => native("mobile_logout"),
  openBrowser: (project, task) =>
    native("mobile_open_browser", {
      project: project ?? null,
      task: task ?? null,
    }),
};
export function runRequest(
  text: string,
  role: string,
  mode: "execute" | "plan",
  key: string,
): RunRequest {
  return {
    input: [{ text }],
    role,
    mode,
    max_turns: 24,
    idempotency_key: key,
    plugin_skill_ids: [],
    session_id: null,
    end_user_id: null,
    research_depth: null,
    research_sources: [],
    plan_action: null,
    branch: null,
  };
}
export function messageOf(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "The request could not be completed.";
}
