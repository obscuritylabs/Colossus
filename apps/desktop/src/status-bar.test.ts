import { describe, expect, it, vi } from "vitest";
import { chatReducer, initialChatState } from "./state";
import type { Run } from "./types";
import {
  backgroundNotificationContent,
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

describe("background notification content", () => {
  const result = (output: string) => ({
    output,
    profile: "default",
    modelProfile: "primary",
    providerProfile: "company",
    model: "sample",
    elapsedSeconds: 1,
  });

  it("uses the saved title and the response retained by the real reducer", async () => {
    const previous = chatReducer(initialChatState, {
      type: "upsert_run",
      run: run(),
    });
    const state = chatReducer(previous, {
      type: "ingest_update",
      update: {
        runId: "run-1",
        sequence: 1,
        createdAt: "2026-09-29T10:01:00Z",
        update: {
          type: "result",
          result: result(
            "## Done\nUpdated **three files**. See [checks](https://example.test).",
          ),
        },
      },
    });
    const loadRun = vi.fn();
    const content = await backgroundNotificationContent(
      state.recentRuns[0]!,
      "Release checklist",
      state.views.get("run-1")!.output,
      loadRun,
    );
    expect(content).toEqual({
      threadTitle: "Release checklist",
      outputPreview: "Done Updated three files. See checks.",
    });
    expect(loadRun).not.toHaveBeenCalled();
  });

  it("loads the matching result for a completed thread outside the view cache", async () => {
    const completed = run({
      status: "completed",
      terminal: { type: "result", result: result("Unopened thread result") },
    });
    const state = chatReducer(initialChatState, {
      type: "replace_recent",
      runs: [completed],
      nextPageToken: "",
    });
    const loadRun = vi
      .fn()
      .mockResolvedValue({ run: completed, pendingInteractions: [] });
    const content = await backgroundNotificationContent(
      state.recentRuns[0]!,
      "",
      "",
      loadRun,
    );
    expect(content).toEqual({
      threadTitle: "First work",
      outputPreview: "Unopened thread result",
    });
    expect(loadRun).toHaveBeenCalledOnce();
  });

  it("keeps notifications usable if the result fetch fails or has no output", async () => {
    for (const loadRun of [
      vi.fn().mockRejectedValue(new Error("offline")),
      vi.fn().mockResolvedValue({
        run: run({
          runId: "other-run",
          terminal: { type: "result", result: result("Unrelated result") },
        }),
        pendingInteractions: [],
      }),
    ]) {
      expect(
        await backgroundNotificationContent(
          run({ status: "completed", title: " " }),
          "",
          "",
          loadRun,
        ),
      ).toEqual({ threadTitle: "Untitled work", outputPreview: "" });
    }
  });

  it("previews released waiting output and failure messages without loading prompts", async () => {
    const loadRun = vi.fn();
    expect(
      (
        await backgroundNotificationContent(
          run({ status: "waiting" }),
          "Input needed",
          "Choose a deployment region.",
          loadRun,
        )
      ).outputPreview,
    ).toBe("Choose a deployment region.");
    expect(
      (
        await backgroundNotificationContent(
          run({
            status: "failed",
            terminal: {
              type: "failure",
              failure: {
                reason: "unavailable",
                message: "Provider unavailable. Try again later.",
                outcomeCertainty: "known",
              },
            },
          }),
          "Check connection",
          "Earlier output",
          loadRun,
        )
      ).outputPreview,
    ).toBe("Provider unavailable. Try again later.");
    expect(loadRun).not.toHaveBeenCalled();
  });

  it("bounds Unicode previews and removes control characters and common Markdown", async () => {
    const content = await backgroundNotificationContent(
      run(),
      "a".repeat(97),
      "```text\n" + "🚀".repeat(200) + "\n```",
      vi.fn(),
    );
    expect(content.threadTitle).toHaveLength(96);
    expect(content.outputPreview.length).toBeLessThanOrEqual(240);
    expect(content.outputPreview.endsWith("…")).toBe(true);
    expect(content.outputPreview).not.toMatch(/[\uD800-\uDBFF]…$/u);
    const clean = await backgroundNotificationContent(
      run(),
      "Ready\u202e\nThread",
      "Done\u0000\t now",
      vi.fn(),
    );
    expect(clean).toEqual({
      threadTitle: "Ready Thread",
      outputPreview: "Done now",
    });
  });
});
