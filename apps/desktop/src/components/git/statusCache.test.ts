import { describe, expect, it } from "vitest";
import type { GitStatus } from "../../git";
import { GitStatusCache } from "./statusCache";

describe("Git workspace snapshots", () => {
  const status: GitStatus = { state: "not_repository", repository: null };

  it("isolates workspaces and does not return a snapshot without a scope", () => {
    const cache = new GitStatusCache();
    cache.set("native:workspace-a", status);
    expect(cache.get("native:workspace-a")).toBe(status);
    expect(cache.get("native:workspace-b")).toBeNull();
    expect(cache.get("fixture:workspace-a")).toBeNull();
    expect(cache.get(null)).toBeNull();
  });

  it("replaces old state, including when repository access needs approval", () => {
    const cache = new GitStatusCache();
    cache.set("workspace", status);
    const restricted: GitStatus = {
      state: "permission_required",
      repository: null,
    };
    cache.set("workspace", restricted);
    expect(cache.get("workspace")).toBe(restricted);
  });

  it("retains the 20 most recently refreshed workspaces", () => {
    const cache = new GitStatusCache();
    for (let i = 0; i < 20; i++) cache.set(`workspace-${i}`, status);
    cache.set("workspace-0", status);
    cache.set("workspace-20", status);
    expect(cache.get("workspace-0")).toBe(status);
    expect(cache.get("workspace-1")).toBeNull();
    expect(cache.get("workspace-20")).toBe(status);
  });
});
