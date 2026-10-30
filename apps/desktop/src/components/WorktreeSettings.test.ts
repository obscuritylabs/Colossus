import { describe, expect, it } from "vitest";
import { CommandFailure } from "../api";
import { worktreeActions, worktreeFailure } from "../worktrees";
import type { ManagedWorktree } from "../worktrees";

const entry: ManagedWorktree = {
  id: "a".repeat(32),
  name: "feature",
  branch: "colossus/feature",
  path: "/owned/trees/id",
  baseCommit: "b".repeat(40),
  state: "active",
  createdAtMs: 1,
  inUse: false,
  dirty: false,
  note: null,
};
describe("worktree action availability", () => {
  it("allows shared opening but never archives or removes a used checkout", () => {
    expect(worktreeActions({ ...entry, inUse: true })).toEqual({
      open: true,
      archive: false,
      remove: false,
    });
    expect(
      worktreeActions({ ...entry, state: "archived", inUse: true }).remove,
    ).toBe(false);
  });
  it("requires a clean archived checkout and blocks unknown or interrupted ownership", () => {
    expect(worktreeActions({ ...entry, state: "archived" }).remove).toBe(true);
    for (const dirty of [true, null])
      expect(
        worktreeActions({ ...entry, state: "archived", dirty }).remove,
      ).toBe(false);
    for (const state of [
      "creating",
      "removing",
      "removed",
      "needs_attention",
    ] as const)
      expect(
        Object.values(worktreeActions({ ...entry, state })).some(Boolean),
      ).toBe(false);
    expect(
      Object.values(
        worktreeActions({ ...entry, note: "Ownership changed" }),
      ).some(Boolean),
    ).toBe(false);
  });
  it("requires explicit refresh after uncertain outcomes without inventing retries", () => {
    const failure = new CommandFailure({
      code: "worktree_outcome_unknown",
      message: "Refresh first",
      retryable: false,
      outcomeUnknown: true,
      violations: [],
    });
    expect(worktreeFailure(failure)).toEqual({
      message: "Refresh first",
      uncertain: true,
    });
    expect(worktreeFailure(new Error("untrusted transport detail"))).toEqual({
      message:
        "Worktree controls could not finish. Refresh the inventory before another action.",
      uncertain: true,
    });
  });
});
