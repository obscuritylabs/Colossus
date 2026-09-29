import { describe, expect, it } from "vitest";
import type { Run } from "./types";
import {
  backgroundRunNotifications,
  backgroundRunSnapshot,
  selectStatusBarPins,
} from "./status-bar";

function run(overrides: Partial<Run> = {}): Run {
  return {
    runId: "run-1",
    sessionId: "session-1",
    title: "First work",
    role: "primary",
    mode: "execute",
    status: "running",
    createdAt: "2026-09-29T10:00:00Z",
    updatedAt: "2026-09-29T10:00:00Z",
    startedAt: "2026-09-29T10:00:00Z",
    finishedAt: null,
    lastSequence: 0,
    pendingInteractionCount: 0,
    terminal: null,
    etag: "etag-1",
    archived: false,
    ...overrides,
  };
}

describe("status bar presentation", () => {
  it("uses the latest run in each pinned session and saved thread titles", () => {
    const pins = selectStatusBarPins(
      ["session-1", "session-2"],
      [
        run(),
        run({
          runId: "run-2",
          updatedAt: "2026-09-29T11:00:00Z",
          title: "Latest work",
        }),
        run({ runId: "run-3", sessionId: "session-2", archived: true }),
      ],
      (_sessionId, fallback) => `Saved: ${fallback}`,
    );
    expect(pins).toEqual([{ runId: "run-2", title: "Saved: Latest work" }]);
  });

  it("only reports live transitions, never already finished work on startup", () => {
    const completed = run({ status: "completed" });
    expect(backgroundRunNotifications(new Map(), [completed])).toEqual([]);
    const running = run();
    const previous = backgroundRunSnapshot([running]);
    expect(
      backgroundRunNotifications(previous, [
        run({ status: "waiting", pendingInteractionCount: 1 }),
      ]),
    ).toEqual([{ kind: "needs_attention", runId: "run-1" }]);
    expect(backgroundRunNotifications(previous, [completed])).toEqual([
      { kind: "work_completed", runId: "run-1" },
    ]);
    expect(
      backgroundRunNotifications(previous, [run({ status: "failed" })]),
    ).toEqual([{ kind: "work_failed", runId: "run-1" }]);
    expect(backgroundRunNotifications(previous, [running])).toEqual([]);
  });
});
