import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Aside } from "../types";
import { AsidePanel } from "./AsidePanel";

const HISTORY: Aside = {
  parentSessionId: "parent-session",
  sourceRunId: "parent-run",
  createdAt: "2026-08-15T00:00:00Z",
  closed: true,
  run: {
    runId: "aside-run",
    sessionId: "aside-session",
    title: "Explain the security boundary",
    role: "primary",
    mode: "execute",
    status: "completed",
    createdAt: "2026-08-15T00:00:00Z",
    updatedAt: "2026-08-15T00:00:01Z",
    startedAt: "2026-08-15T00:00:00Z",
    finishedAt: "2026-08-15T00:00:01Z",
    lastSequence: 2,
    pendingInteractionCount: 0,
    terminal: null,
    etag: "aside-etag",
    archived: true,
  },
};

describe("AsidePanel", () => {
  it("shows selected context and parent-scoped archived history", () => {
    const markup = renderToStaticMarkup(
      createElement(AsidePanel, {
        draft: {
          sourceRunId: "parent-run",
          quote:
            "Native enforcement prevents another Space from being queried.",
        },
        view: undefined,
        conversationViews: [],
        history: [HISTORY],
        busy: false,
        error: null,
        readOnly: false,
        onCreate: vi.fn(async () => true),
        onContinue: vi.fn(async () => true),
        onOpen: vi.fn(async () => undefined),
        onNew: vi.fn(),
        onRespond: vi.fn(async () => undefined),
        onClose: vi.fn(async () => true),
        onDismiss: vi.fn(),
      }),
    );

    expect(markup).toContain("Explore without changing the main thread");
    expect(markup).toContain("Native enforcement prevents another Space");
    expect(markup).toContain("Past Asides");
    expect(markup).toContain("Thread messages included · tool traces omitted");
    expect(markup).toContain("Explain the security boundary");
    expect(markup).toContain("Archived");
  });
});
