import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { ScheduleActivityCard } from "./ScheduleActivityCard";
import type { ToolActivity } from "../types";

function render(state: ToolActivity["state"], preview?: string, id = "daily") {
  return renderToStaticMarkup(
    createElement(ScheduleActivityCard, {
      activity: {
        toolName: "workflow.schedule.create",
        state,
        summary: "Schedule request",
        preview: preview ?? null,
        callId: "schedule-call",
      },
      input: JSON.stringify({ schedule_id: id }),
      onInspect: vi.fn(),
    }),
  );
}
it("links released canonical schedules and their independent runs", () => {
  const html = render(
    "completed",
    JSON.stringify({
      record: {
        schedule_id: "daily",
        enabled: false,
        cadence_seconds: 86400,
        last_run_id: "workflow-run-1",
      },
    }),
  );
  expect(html).toContain("Paused · Every 24 hours");
  expect(html).toContain("Inspect schedule");
  expect(html).toContain("Inspect last workflow run");
});
it("does not invent allocation or run links from waiting or uncertain activity", () => {
  for (const state of ["waiting_approval", "outcome_unknown"] as const) {
    const html = render(
      state,
      JSON.stringify({
        record: {
          schedule_id: "daily",
          enabled: true,
          last_run_id: "fabricated",
        },
      }),
    );
    expect(html).toContain("Schedule request");
    expect(html).not.toContain("Inspect last workflow run");
  }
  expect(render("completed", "truncated preview", "../../private")).toBe("");
});
