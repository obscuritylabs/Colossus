import { describe, expect, it } from "vitest";
import {
  agentHref,
  globalHref,
  parseRoute,
  projectHref,
  safeReturnPath,
  taskHref,
  threadHref,
} from "./routes";
const origin = "https://control.example";
describe("Control Plane permalinks", () => {
  it("round-trips major pages and encoded, exact scoped resource identifiers", () => {
    expect(parseRoute(globalHref("fleet", "project:a"), origin)).toEqual({
      kind: "global",
      surface: "fleet",
      project: "project:a",
    });
    expect(
      parseRoute(globalHref("admin", undefined, "settings"), origin),
    ).toEqual({ kind: "global", surface: "admin", view: "settings" });
    expect(parseRoute(projectHref("project:a", "access"), origin)).toEqual({
      kind: "project",
      project: "project:a",
      view: "access",
    });
    expect(
      parseRoute(agentHref("project:a", "node:b", "policy", true), origin),
    ).toEqual({
      kind: "agent",
      project: "project:a",
      node: "node:b",
      view: "policy",
      compose: true,
    });
    expect(parseRoute(threadHref("project:a", "thread:c"), origin)).toEqual({
      kind: "thread",
      project: "project:a",
      thread: "thread:c",
    });
    expect(parseRoute(taskHref("project:a", "task:d"), origin)).toEqual({
      kind: "task",
      project: "project:a",
      task: "task:d",
    });
  });
  it("rejects malformed, ambiguous, oversized, and cross-origin destinations", () => {
    for (const href of [
      "https://other.example/fleet",
      "//other.example/fleet",
      "/projects/p/threads/%2Fsecret",
      "/projects/p/threads/%ZZ",
      "/projects/p/threads/",
      "/fleet?project=p&project=q",
      "/fleet?permissions=execute",
      "/projects/p/agents/n/policy?compose=0",
      `/projects/p/threads/${"x".repeat(257)}`,
      `/fleet?${"x".repeat(2048)}`,
    ]) {
      expect(parseRoute(href, origin).kind, href).toBe("invalid");
      expect(safeReturnPath(href, origin), href).toBeNull();
    }
    expect(
      safeReturnPath(`${origin}/projects/p/threads/t#ignored`, origin),
    ).toBe("/projects/p/threads/t");
  });
});
