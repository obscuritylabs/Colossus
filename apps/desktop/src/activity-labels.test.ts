import { describe, expect, it } from "vitest";

import type { ToolActivity } from "./types";
import { presentNotice, presentToolActivity } from "./activity-labels";

function activity(
  toolName: string,
  state: ToolActivity["state"] = "completed",
  summary = `tool execution ${state} at turn 1`,
): ToolActivity {
  return { callId: "call-1", toolName, state, summary };
}

describe("activity labels", () => {
  it("names filesystem work from released structured input", () => {
    expect(
      presentToolActivity(
        activity("filesystem.write"),
        '{"path":"notes/sample.txt","mode":"create"}',
      ),
    ).toEqual({ kind: "write", title: "Created sample.txt" });
    expect(
      presentToolActivity(
        activity("filesystem.read"),
        '{"path":"apps/desktop/src/WorkSidebar.tsx"}',
      ).title,
    ).toBe("Read WorkSidebar.tsx");
    expect(
      presentToolActivity(activity("repo.read_many"), '{"paths":["a","b"]}')
        .title,
    ).toBe("Read 2 files");
    expect(
      presentToolActivity(
        activity("filesystem.list"),
        '{"path":"apps/desktop"}',
      ).title,
    ).toBe("Listed files in desktop");
    expect(
      presentToolActivity(
        activity("filesystem.search"),
        '{"pattern":"WorkSidebar","path":"apps/desktop/src"}',
      ).title,
    ).toBe("Searched “WorkSidebar” in src");
  });

  it("names searches and shell commands without generic turn copy", () => {
    expect(
      presentToolActivity(
        activity("repo.search"),
        '{"query":"View all|Load more"}',
      ).title,
    ).toBe("Searched “View all|Load more”");
    expect(
      presentToolActivity(
        activity("shell.run", "started"),
        '{"command":"git status --short"}',
      ).title,
    ).toBe("Running git status --short");
    expect(
      presentToolActivity(
        activity("shell.run"),
        '{"argv":["cargo","test","-p","desktop"]}',
      ).title,
    ).toBe("Ran cargo test -p desktop");
    expect(presentToolActivity(activity("repo.map")).title).toBe(
      "Mapped repository structure",
    );
    expect(presentToolActivity(activity("web.search")).title).toBe(
      "Searched the web",
    );
    expect(
      presentToolActivity(activity("web.search"), '{"query":"Colossus"}').title,
    ).toBe("Searched the web for “Colossus”");
  });

  it("names patch lifecycle actions from their target file", () => {
    expect(
      presentToolActivity(activity("patch.preview"), '{"path":"src/App.tsx"}')
        .title,
    ).toBe("Previewed changes to App.tsx");
    expect(
      presentToolActivity(activity("patch.apply"), '{"path":"src/App.tsx"}')
        .title,
    ).toBe("Updated App.tsx");
    expect(
      presentToolActivity(activity("patch.reverse"), '{"path":"src/App.tsx"}')
        .title,
    ).toBe("Reverted App.tsx");
  });

  it("preserves a useful released summary for unknown tools", () => {
    expect(
      presentToolActivity({
        ...activity("extension.review"),
        summary: "Reviewed deployment readiness",
      }).title,
    ).toBe("Reviewed deployment readiness");
    expect(presentToolActivity(activity("extension.review")).title).toBe(
      "Used extension review",
    );
  });

  it("translates research event identifiers and awkward details", () => {
    expect(
      presentNotice(
        "research.planning.started",
        "Planning bounded research queries.",
      ),
    ).toEqual({
      title: "Planning research",
      detail: "Preparing focused, bounded research queries.",
      kind: "research",
    });
    expect(
      presentNotice(
        "research.collecting.completed",
        "released 20 repository source(s)",
      ).detail,
    ).toBe("Added 20 repository sources.");
    expect(
      presentNotice(
        "research.collecting.skipped",
        "bounded research worker or source budget exhausted",
      ).detail,
    ).toBe("Source or worker limit reached.");
  });
});
