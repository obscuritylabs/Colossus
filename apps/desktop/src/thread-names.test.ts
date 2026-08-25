import { describe, expect, it } from "vitest";

import {
  MAX_NAMED_THREADS_PER_WORKSPACE,
  normalizeThreadName,
  parseStoredThreadNames,
  setThreadName,
  threadNameForWorkspace,
} from "./thread-names";

describe("thread names", () => {
  it("parses bounded names and rejects unsafe display text", () => {
    const names = parseStoredThreadNames(
      JSON.stringify([
        {
          spaceId: "space-a",
          threads: [
            { sessionId: "session-a", name: " Release review " },
            { sessionId: "session-b", name: "spoof\u202ereview" },
          ],
        },
      ]),
    );

    expect(names).toEqual([
      {
        spaceId: "space-a",
        threads: [{ sessionId: "session-a", name: "Release review" }],
      },
    ]);
    expect(parseStoredThreadNames("not json")).toEqual([]);
    expect(normalizeThreadName("\nunsafe")).toBeNull();
  });

  it("renames sessions independently per Workspace and keeps recent names first", () => {
    let names = setThreadName([], "space-a", "session-a", "First name");
    names = setThreadName(names, "space-b", "session-a", "Other Workspace");
    names = setThreadName(names, "space-a", "session-a", "Updated name");

    expect(threadNameForWorkspace(names, "space-a", "session-a")).toBe(
      "Updated name",
    );
    expect(threadNameForWorkspace(names, "space-b", "session-a")).toBe(
      "Other Workspace",
    );
  });

  it("bounds the number of retained names per Workspace", () => {
    let names = [] as ReturnType<typeof parseStoredThreadNames>;
    for (
      let index = 0;
      index < MAX_NAMED_THREADS_PER_WORKSPACE + 4;
      index += 1
    ) {
      names = setThreadName(
        names,
        "space-a",
        `session-${index}`,
        `Thread ${index}`,
      );
    }

    expect(names[0]?.threads).toHaveLength(MAX_NAMED_THREADS_PER_WORKSPACE);
    expect(names[0]?.threads[0]?.name).toBe("Thread 131");
  });
});
