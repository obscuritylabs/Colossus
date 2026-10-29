import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ReleasedRunActivity } from "@colossus/ui/conversation";

describe("shared released tool labels", () => {
  it.each([
    ["repo.map", "Mapped repository structure"],
    ["mcp.call", "Called GitLab · list_issues"],
  ])(
    "shows the runtime summary for %s without reinterpreting released details",
    (name, summary) => {
      const markup = renderToStaticMarkup(
        createElement(ReleasedRunActivity, {
          label: "Released activity",
          status: "completed",
          updates: [
            {
              run_id: "run-1",
              sequence: 1,
              created_at: "2026-10-29T10:00:00Z",
              update: {
                tool_activity: {
                  call_id: "call-1",
                  tool_name: name,
                  state: "completed",
                  summary,
                  input:
                    '{"server":"not-the-summary","arguments":{"server":"payload-server"}}',
                  preview: "Released result",
                },
              },
            },
          ],
        }),
      );
      const heading = markup.match(
        /class="shared-activity-item shared-activity-tool"[^>]*><summary>(.*?)<\/summary>/su,
      )?.[1];
      expect(heading).toContain(summary);
      expect(heading).not.toContain("payload-server");
      expect(heading).not.toContain("not-the-summary");
      expect(markup).toContain("Released result");
      expect(markup).toContain(name);
    },
  );

  it("retains a generic legacy label", () => {
    const markup = renderToStaticMarkup(
      createElement(ReleasedRunActivity, {
        label: "Released activity",
        status: "running",
        updates: [
          {
            run_id: "run-1",
            sequence: 1,
            created_at: "",
            update: {
              tool_activity: {
                call_id: "call-1",
                tool_name: "repo.map",
                state: "started",
                summary: "tool execution started at turn 1",
              },
            },
          },
        ],
      }),
    );
    expect(markup).toContain("Tool activity · repo.map");
    expect(markup).toContain('data-state="started"');
  });

  it("renders canonical labels as escaped plain text", () => {
    const summary = '<img src=x onerror="alert(1)">';
    const markup = renderToStaticMarkup(
      createElement(ReleasedRunActivity, {
        label: "Released activity",
        status: "completed",
        updates: [
          {
            run_id: "run-1",
            sequence: 1,
            created_at: "",
            update: {
              tool_activity: {
                call_id: "call-1",
                tool_name: "extension.review",
                state: "completed",
                summary,
              },
            },
          },
        ],
      }),
    );
    const heading = markup.match(
      /class="shared-activity-item shared-activity-tool"[^>]*><summary>(.*?)<\/summary>/su,
    )?.[1];
    expect(heading).toContain("&lt;img");
    expect(heading).not.toContain("<img");
  });
});
