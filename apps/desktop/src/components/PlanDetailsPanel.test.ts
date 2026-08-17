import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { PlanDetailsPanel } from "./PlanDetailsPanel";

describe("PlanDetailsPanel", () => {
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
