import { invoke } from "@tauri-apps/api/core";
import { CommandFailure, normalizeCommandError } from "./api";
import type { DesktopStatus } from "./types";

export interface WorktreePreferences {
  directory: string;
  settings: { branchPrefix: string; openAfterCreate: boolean };
}
export interface ManagedWorktree {
  id: string;
  name: string;
  branch: string;
  baseCommit: string;
  path: string;
  state:
    | "creating"
    | "active"
    | "archived"
    | "removing"
    | "removed"
    | "needs_attention";
  createdAtMs: number;
  inUse: boolean;
  dirty: boolean | null;
  note: string | null;
}
export interface WorktreeCatalog {
  repositoryName: string;
  repositoryId: string;
  storageRoot: string;
  settings: WorktreePreferences["settings"];
  worktrees: ManagedWorktree[];
  maxWorktrees: number;
}
export type WorktreeAction =
  | {
      kind: "create";
      request: {
        name: string;
        branch: string | null;
        base: string | null;
        existingBranch: boolean;
      };
    }
  | { kind: "open" | "archive" | "remove"; id: string };
export interface WorktreeSnapshot {
  state: "permission_required" | "ready";
  catalog: WorktreeCatalog | null;
}
export interface WorktreeActionResult {
  catalog: WorktreeCatalog;
  opened: DesktopStatus | null;
}
export interface WorktreesApi {
  preferences(): Promise<WorktreePreferences>;
  save(settings: WorktreePreferences["settings"]): Promise<WorktreePreferences>;
  choose(reset: boolean): Promise<WorktreePreferences>;
  list(
    workspaceId: string,
    approveRepository: boolean,
  ): Promise<WorktreeSnapshot>;
  action(
    workspaceId: string,
    action: WorktreeAction,
  ): Promise<WorktreeActionResult>;
}
async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (failure) {
    throw new CommandFailure(normalizeCommandError(failure));
  }
}
export const nativeWorktreesApi: WorktreesApi = {
  preferences: () => call("get_worktree_preferences"),
  save: (settings) => call("save_worktree_preferences", { settings }),
  choose: (reset) => call("choose_worktree_directory", { reset }),
  list: (workspaceId, approveRepository) =>
    call("get_workspace_worktrees", {
      request: { workspaceId, approveRepository },
    }),
  action: (workspaceId, action) =>
    call("workspace_worktree_action", { request: { workspaceId, action } }),
};

export function worktreeActions(entry: ManagedWorktree): {
  open: boolean;
  archive: boolean;
  remove: boolean;
} {
  const usable = !entry.note;
  return {
    open: usable && (entry.state === "active" || entry.state === "archived"),
    archive: usable && entry.state === "active" && !entry.inUse,
    remove:
      usable &&
      entry.state === "archived" &&
      !entry.inUse &&
      entry.dirty === false,
  };
}
export function worktreeFailure(failure: unknown): {
  message: string;
  uncertain: boolean;
} {
  return failure instanceof CommandFailure
    ? { message: failure.message, uncertain: failure.detail.outcomeUnknown }
    : {
        message:
          "Worktree controls could not finish. Refresh the inventory before another action.",
        uncertain: true,
      };
}
