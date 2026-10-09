import { describe, expect, it } from "vitest";
import {
  canForkThread,
  createThreadForkDraft,
  defaultForkTitle,
  parseThreadForkDrafts,
  threadForkBranch,
  threadForkDraftRun,
} from "./thread-fork";
import type { Run } from "./types";

const source: Run = {
  runId: "owned-source-run",
  sessionId: "parent-session",
  role: "primary",
  title: "Parent",
  mode: "plan",
  status: "completed",
  archived: false,
  createdAt: "2026-10-06T00:00:00Z",
  updatedAt: "2026-10-06T00:00:00Z",
  startedAt: null,
  finishedAt: null,
  lastSequence: 0,
  pendingInteractionCount: 0,
  terminal: null,
  etag: "source",
};
describe("thread forking", () => {
  it("creates only local metadata until the full composer sends", () => {
    const draft = createThreadForkDraft(
      source,
      "workspace",
      "  Another approach  ",
    );
    expect(draft.title).toBe("Another approach");
    expect(draft).not.toHaveProperty("prompt");
    expect(draft).not.toHaveProperty("transcript");
    expect(threadForkBranch(draft)).toEqual({
      sourceRunId: "owned-source-run",
      kind: "thread",
    });
    const summary = threadForkDraftRun(draft);
    expect(summary.sessionId).not.toBe(source.sessionId);
    expect(summary.terminal).toBeNull();
    expect(canForkThread(summary)).toBe(false);
    expect(source.sessionId).toBe("parent-session");
  });
  it("rejects unfinished/archived sources and invalid titles", () => {
    for (const status of ["running", "queued", "waiting"] as const) {
      expect(canForkThread({ ...source, status })).toBe(false);
      expect(() =>
        createThreadForkDraft({ ...source, status }, "workspace", "Fork"),
      ).toThrow();
    }
    expect(canForkThread({ ...source, archived: true })).toBe(false);
    expect(() => createThreadForkDraft(source, "workspace", "  ")).toThrow();
    expect([...defaultForkTitle("x".repeat(160))]).toHaveLength(80);
  });
  it("restores bounded metadata without trusting extra transcript or request fields", () => {
    const draft = createThreadForkDraft(source, "workspace", "Fork");
    expect(
      parseThreadForkDrafts(
        JSON.stringify([
          { ...draft, prompt: "must not send", transcript: "untrusted" },
          draft,
        ]),
      ),
    ).toEqual([draft]);
    expect(
      parseThreadForkDrafts(
        JSON.stringify([{ ...draft, spaceId: "bad\nworkspace" }]),
      ),
    ).toEqual([]);
    expect(parseThreadForkDrafts("x".repeat(65_537))).toEqual([]);
    const materialized = {
      ...draft,
      materializedSessionId: "separate-session",
    };
    expect(parseThreadForkDrafts(JSON.stringify([materialized]))).toEqual([
      materialized,
    ]);
  });
});
