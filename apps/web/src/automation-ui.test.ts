import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  AutomationExampleGallery,
  AutomationInventory,
  AutomationOverview,
  AutomationSurface,
  AutomationWelcome,
} from "@colossus/ui/automations";
import { Button } from "@colossus/ui";

describe("shared automation presentation in the web host", () => {
  it("renders the shared creation surface without a native runtime", () => {
    const markup = renderToStaticMarkup(
      createElement(AutomationSurface, {
        label: "Web workflows",
        children: createElement(AutomationWelcome, {
          title: "Create a workflow",
          description: "Describe the work to repeat.",
          icon: null,
          action: createElement(
            Button,
            { disabled: true },
            "Create with agent",
          ),
          features: [],
        }),
      }),
    );
    expect(markup).toContain('aria-label="Web workflows"');
    expect(markup).toContain("Create a workflow");
    expect(markup).toContain('disabled=""');
  });
  it("keeps examples, timing summaries, and schedule inventory portable", () => {
    const markup = renderToStaticMarkup(
      createElement(
        "div",
        null,
        createElement(AutomationExampleGallery, {
          title: "Start with an example",
          description: "Choose a task.",
          disabled: true,
          onCreate: () => {},
          examples: [
            {
              id: "health",
              name: "Health check",
              description: "Inspect changes.",
              timing: "Every morning",
            },
          ],
        }),
        createElement(AutomationOverview, {
          items: [
            { id: "next", label: "Next run", value: "Tomorrow at 09:00" },
          ],
        }),
        createElement(AutomationInventory, {
          busy: false,
          onInspect: () => {},
          rows: [
            {
              id: "health",
              name: "Health check",
              kind: "Agent task",
              actionLabel: "Inspect Health check",
              searchText: "Health check",
              repeat: "Daily",
              nextOccurrence: "Tomorrow",
              nextOccurrenceTitle: "Tomorrow at 09:00",
              occurrenceLabel: "Retained occurrence",
              status: "paused",
            },
          ],
        }),
      ),
    );
    expect(markup).toContain('aria-label="Create Health check with agent"');
    expect(markup).toContain("Tomorrow at 09:00");
    expect(markup).toContain('data-slot="table"');
    expect(markup).toContain('data-slot="input"');
    expect(markup).toContain('data-slot="badge"');
    expect(markup).toContain('data-state="paused"');
  });
});
