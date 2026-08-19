import { describe, expect, it } from "vitest";

import {
  MAX_PINNED_THREADS_PER_SPACE,
  parseStoredThreadPins,
  pinnedThreadIdsForSpace,
  readStoredThreadPins,
  setThreadPinned,
} from "./thread-pins";

describe("thread pins", () => {
  it("parses only bounded, unique Space and session identifiers", () => {
    const pins = parseStoredThreadPins(
      JSON.stringify([
        {
          spaceId: "space-a",
          sessionIds: ["session-a", "session-a", "\nunsafe"],
        },
        { spaceId: "space-a", sessionIds: ["ignored-duplicate-space"] },
        { spaceId: "space-empty", sessionIds: [] },
      ]),
    );

    expect(pins).toEqual([{ spaceId: "space-a", sessionIds: ["session-a"] }]);
    expect(parseStoredThreadPins("not json")).toEqual([]);
  });

  it("toggles pins independently per Space and keeps the newest pin first", () => {
    let pins = setThreadPinned([], "space-a", "session-a", true);
    pins = setThreadPinned(pins, "space-b", "session-b", true);
    pins = setThreadPinned(pins, "space-a", "session-c", true);

    expect(pinnedThreadIdsForSpace(pins, "space-a")).toEqual([
      "session-c",
      "session-a",
    ]);
    expect(pinnedThreadIdsForSpace(pins, "space-b")).toEqual(["session-b"]);
    expect(
      pinnedThreadIdsForSpace(
        setThreadPinned(pins, "space-a", "session-c", false),
        "space-a",
      ),
    ).toEqual(["session-a"]);
  });

  it("bounds the number of retained pins per Space", () => {
    let pins = [] as ReturnType<typeof parseStoredThreadPins>;
    for (let index = 0; index < MAX_PINNED_THREADS_PER_SPACE + 4; index += 1) {
      pins = setThreadPinned(pins, "space-a", `session-${index}`, true);
    }

    expect(pinnedThreadIdsForSpace(pins, "space-a")).toHaveLength(
      MAX_PINNED_THREADS_PER_SPACE,
    );
    expect(pinnedThreadIdsForSpace(pins, "space-a")[0]).toBe("session-35");
  });

  it("uses a caller-provided default only when stored pins are absent", () => {
    const fallback = [{ spaceId: "space-a", sessionIds: ["session-a"] }];

    expect(readStoredThreadPins(fallback)).toEqual(fallback);
  });
});
