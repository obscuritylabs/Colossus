import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { RunView, StreamState } from "../state";
import type { Run, RunStatus } from "../types";
import { RunTimeline } from "./RunTimeline";

function renderOutput(
  output: string,
  status: RunStatus,
  streamState: StreamState,
): string {
  const run: Run = {
    runId: "run-markdown-test",
    sessionId: "session-markdown-test",
    role: "primary",
    mode: "execute",
    status,
    createdAt: "2026-07-21T12:00:00Z",
    updatedAt: "2026-07-21T12:00:01Z",
    startedAt: "2026-07-21T12:00:00Z",
    finishedAt: status === "completed" ? "2026-07-21T12:00:01Z" : null,
    lastSequence: 0,
    pendingInteractionCount: 0,
    terminal: null,
    etag: "etag-markdown-test",
    selectedSkills: [],
  };
  const view: RunView = {
    run,
    localPrompt: null,
    output,
    updates: [],
    seenSequences: new Set(),
    lastSequence: 0,
    pendingInteractions: [],
    usage: null,
    streamState,
    streamError: null,
  };

  return renderToStaticMarkup(createElement(RunTimeline, { view }));
}

describe("RunTimeline assistant output", () => {
  it("renders completed assistant responses as Markdown", () => {
    const markup = renderOutput(
      "# Ready\n\n- **security** checked",
      "completed",
      "complete",
    );

    expect(markup).toContain('class="markdown-content"');
    expect(markup).toContain('<h3 class="feed-entry-title">Colossus</h3>');
    expect(markup).toContain("<h4>Ready</h4>");
    expect(markup).toContain("<strong>security</strong>");
  });

  it("keeps active streamed output plain until the response is complete", () => {
    const markup = renderOutput("# Still streaming", "running", "watching");

    expect(markup).toContain("# Still streaming");
    expect(markup).not.toContain('class="markdown-content"');
    expect(markup).not.toContain("<h3>Still streaming</h3>");
    expect(markup).toContain('class="stream-caret"');
  });
});
