import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { PlanDetailsPanel } from "./PlanDetailsPanel";

describe("PlanDetailsPanel", () => {
  it.each([
    ["executed", 3, true],
    ["approved", 2, true],
    ["discarded", 2, true],
    ["draft", 0, true],
    ["draft", 1, false],
  ] as const)(
    "disables revision for %s revision %i with continuation %s",
    (status, revision, continuationAvailable) => {
      const markup = renderToStaticMarkup(
        createElement(PlanDetailsPanel, {
          plan: {
            planId: "plan-1",
            revision,
            status,
            sourceRunId: "run-1",
            sourceRunTitle: "Plan",
            runIndex: 1,
            createdAt: "2026-08-16T19:06:00Z",
            cancelled: false,
            output: "Plan saved.",
          },
          sessionId: "session-1",
          continuationAvailable,
          workflowAvailable: true,
          onBack: vi.fn(),
          onRevise: vi.fn(),
          onOpenWorkflow: vi.fn(),
        }),
      );
      expect(markup).toMatch(
        /<button[^>]*disabled=""[^>]*>.*?Revise in chat<\/button>/,
      );
    },
  );

  it("renders a released plan as readable Markdown with plan actions", () => {
    const markup = renderToStaticMarkup(
      createElement(PlanDetailsPanel, {
        plan: {
          planId: "plan-1",
          revision: 2,
          status: "draft",
          sourceRunId: "run-4",
          sourceRunTitle: "Harden the desktop bootstrap",
          runIndex: 4,
          createdAt: "2026-08-16T19:06:00Z",
          cancelled: false,
          output: "## Implementation\n\n- Map the lifecycle\n- Add tests",
        },
        sessionId: "session-1",
        continuationAvailable: true,
        workflowAvailable: true,
        onBack: vi.fn(),
        onRevise: vi.fn(),
        onOpenWorkflow: vi.fn(),
      }),
    );

    expect(markup).toContain("Harden the desktop bootstrap");
    expect(markup).toContain("Run 4 · Revision 2");
    expect(markup).toContain("<h4>Implementation</h4>");
    expect(markup).toContain("Revise in chat");
    expect(markup).toContain("Open workflow");
    expect(markup).toContain('data-aside-source-run-id="run-4"');
  });
});
