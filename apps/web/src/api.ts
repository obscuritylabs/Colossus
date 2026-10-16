export {
  taskStatus,
  terminalStatuses,
  statusLabel,
  visibleOutput,
} from "@colossus/control-plane-client";
export type * from "@colossus/control-plane-client";
import type { CommandReceipt, Task } from "@colossus/control-plane-client";

export class ApiFailure extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
  }
}
// A successful HTTP mutation acknowledges the queue. Only the runtime receipt
// confirms the operation; an offline runtime leaves it durably queued.
export async function awaitReceipt(
  taskPath: string,
  command: CommandReceipt,
  signal?: AbortSignal,
): Promise<boolean> {
  const deadline = Date.now() + 10000;
  for (;;) {
    if (command.reply) {
      if (command.reply.kind === "failed")
        throw new Error(
          command.reply.error?.message ?? "The runtime rejected this action.",
        );
      return true;
    }
    if (Date.now() >= deadline) return false;
    await new Promise<void>((resolve) => setTimeout(resolve, 300));
    command = (
      await request<{ command: CommandReceipt }>(
        `${taskPath}/commands/${encodeURIComponent(command.command_id)}`,
        undefined,
        signal,
      )
    ).command;
  }
}
const messages: Record<string, string> = {
  permission_denied:
    "Your session expired or this project permission is unavailable.",
  conflict:
    "The state changed. Refresh and review the current task before retrying.",
  storage:
    "The control plane is temporarily unavailable. Your accepted work remains on its runtime.",
  resource_exhausted:
    "A resource limit was reached. Wait for active work to finish before retrying.",
  invalid_argument: "Check the request fields and try again.",
  not_found: "This resource is no longer available in the selected project.",
};
export async function request<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
  method?: "PATCH" | "DELETE",
): Promise<T> {
  const response = await fetch(path, {
    method: method ?? (body === undefined ? "GET" : "POST"),
    credentials: "same-origin",
    headers:
      body === undefined
        ? method === "DELETE"
          ? { "X-Colossus-CSRF": "1" }
          : {}
        : { "Content-Type": "application/json", "X-Colossus-CSRF": "1" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    ...(signal ? { signal } : {}),
  });
  if (!response.ok) {
    // The web host reconciles identity after authorization failures. A project
    // denial may only remove one role; it must not blindly log the user out.
    if (
      (response.status === 401 || response.status === 403) &&
      typeof window !== "undefined" &&
      path.startsWith("/api/") &&
      !["/api/me", "/api/auth/config", "/api/settings"].includes(path)
    ) {
      window.dispatchEvent(new Event("colossus:web:reconcile-identity"));
    }
    const error = (await response
      .json()
      .catch(() => ({ error: "storage" }))) as { error?: string };
    throw new ApiFailure(
      response.status,
      messages[error.error ?? "storage"] ??
        "The request could not be completed.",
    );
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}
export function projectPath(project: string) {
  return `/api/projects/${encodeURIComponent(project)}`;
}
export function taskTitle(task: Task) {
  return (
    task.snapshot?.run.title ||
    task.request.input.map((part) => part.text).join(" ") ||
    "Untitled task"
  );
}
