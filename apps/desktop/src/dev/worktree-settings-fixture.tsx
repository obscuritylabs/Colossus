/** Browser-only synthetic presentation fixture. No native or Git effects. */
import { createRoot } from "react-dom/client";
import { useState } from "react";
import {
  AppearanceProvider,
  initializeAppearance,
} from "../theme/AppearanceProvider";
import { WorktreeSettings } from "../components/WorktreeSettings";
import { CommandFailure } from "../api";
import type {
  ManagedWorktree,
  WorktreeCatalog,
  WorktreePreferences,
  WorktreesApi,
} from "../worktrees";
import type { DesktopStatus } from "../types";
import "@colossus/ui/styles/theme.css";
import "@colossus/ui/styles/controls.css";
import "../styles.css";

if (!import.meta.env.DEV || location.pathname !== "/worktree-component-fixture")
  throw new Error("Worktree presentation fixture unavailable.");
const query = new URLSearchParams(location.search);
const directory = "C:\\OwnedFixture\\ColossusWorktrees";
let preferences: WorktreePreferences = {
  directory,
  settings: { branchPrefix: "colossus/", openAfterCreate: true },
};
let catalog: WorktreeCatalog = {
  repositoryName: "Owned repository",
  repositoryId: "b".repeat(64),
  storageRoot: directory,
  settings: preferences.settings,
  worktrees: [],
  maxWorktrees: 128,
};
let approved = false;
const fixture = {
  calls: [] as { kind: string; request?: unknown }[],
  failNext: false,
  opened: 0,
  dirty(id: string, value: boolean) {
    const entry = catalog.worktrees.find((entry) => entry.id === id);
    if (entry) entry.dirty = value;
  },
};
Object.assign(window, { worktreeFixture: fixture });
function failure(message: string, unknown = false): never {
  throw new CommandFailure({
    code: unknown ? "worktree_outcome_unknown" : "worktree_conflict",
    message,
    retryable: false,
    outcomeUnknown: unknown,
    violations: [],
  });
}
const api: WorktreesApi = {
  async preferences() {
    return structuredClone(preferences);
  },
  async save(settings) {
    fixture.calls.push({ kind: "save", request: settings });
    preferences.settings = structuredClone(settings);
    catalog.settings = structuredClone(settings);
    return structuredClone(preferences);
  },
  async choose(reset) {
    fixture.calls.push({ kind: "choose", request: reset });
    preferences.directory = reset
      ? directory
      : "C:\\AnotherOwnedFixture\\ColossusWorktrees";
    catalog.storageRoot = preferences.directory;
    return structuredClone(preferences);
  },
  async list(workspaceId, approveRepository) {
    fixture.calls.push({
      kind: "list",
      request: { workspaceId, approveRepository },
    });
    if (approveRepository) approved = true;
    return approved
      ? { state: "ready", catalog: structuredClone(catalog) }
      : { state: "permission_required", catalog: null };
  },
  async action(workspaceId, action) {
    fixture.calls.push({ kind: action.kind, request: { workspaceId, action } });
    let opened = false;
    if (action.kind === "create") {
      const entry: ManagedWorktree = {
        id: (catalog.worktrees.length + 1).toString(16).padStart(32, "0"),
        name: action.request.name,
        branch:
          action.request.branch ??
          `${preferences.settings.branchPrefix}${action.request.name}`,
        path: `${preferences.directory}\\trees\\${action.request.name}`,
        baseCommit: "c".repeat(40),
        state: "active",
        createdAtMs: 1,
        inUse: false,
        dirty: false,
        note: null,
      };
      catalog.worktrees.unshift(entry);
      opened = preferences.settings.openAfterCreate;
    } else {
      const entry = catalog.worktrees.find((entry) => entry.id === action.id);
      if (!entry) failure("Choose a worktree from the inventory.");
      if (action.kind === "archive") entry.state = "archived";
      if (action.kind === "open") {
        entry.state = "active";
        opened = true;
      }
      if (action.kind === "remove") {
        if (entry.dirty !== false)
          failure("Preserve local files before removing.");
        entry.state = "removed";
      }
    }
    if (fixture.failNext) {
      fixture.failNext = false;
      failure(
        "Operation outcome is unknown. Refresh before another change.",
        true,
      );
    }
    return {
      catalog: structuredClone(catalog),
      opened: opened ? ({} as DesktopStatus) : null,
    };
  },
};
function Fixture() {
  const [count, setCount] = useState(0);
  return (
    <main style={{ padding: 20, maxWidth: 1100, margin: "0 auto" }}>
      <WorktreeSettings
        workspaceId={query.has("external") ? null : "fixture-workspace"}
        api={api}
        onOpened={() => {
          fixture.opened++;
          setCount(fixture.opened);
        }}
      />
      <span aria-label="Workspaces opened">{count}</span>
    </main>
  );
}
const root = document.getElementById("root");
if (!root) throw new Error("Worktree fixture root missing.");
createRoot(root).render(
  <AppearanceProvider initialPreference={initializeAppearance()}>
    <Fixture />
  </AppearanceProvider>,
);
