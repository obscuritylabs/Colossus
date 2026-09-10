import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Interaction } from "../types";
import { InteractionCard } from "./InteractionCard";

function promptInteraction(overrides: Partial<Interaction> = {}): Interaction {
  return {
    interactionId: "interaction-question",
    runId: "run-question",
    kind: "user_prompt",
    status: "pending",
    createdAt: "2026-07-30T12:00:00Z",
    expiresAt: "2026-07-30T12:30:00Z",
    respondableByCaller: true,
    etag: "etag-question",
    content: {
      type: "user_prompt",
      question: "What is your favorite programming language?",
      choices: [
        { choiceId: "javascript", label: "JavaScript" },
        { choiceId: "python", label: "Python" },
        { choiceId: "go", label: "Go" },
        { choiceId: "rust", label: "Rust" },
      ],
      allowFreeForm: false,
    },
    ...overrides,
  };
}

function renderInteraction(interaction: Interaction): string {
  return renderToStaticMarkup(
    createElement(InteractionCard, {
      interaction,
      onRespond: vi.fn(),
    }),
  );
}

describe("InteractionCard user prompt", () => {
  it("keeps the question, bounded answer body, and action footer separate", () => {
    const markup = renderInteraction(promptInteraction());

    expect(markup).toContain("interaction-card prompt-card");
    expect(markup).toContain("interaction-heading prompt-heading");
    expect(markup).toContain('class="interaction-body"');
    expect(markup).toContain("interaction-actions prompt-actions");
    expect(markup).toContain("Select one response");
    expect(markup).toContain("Send response");
    expect(markup.match(/class="choice"/g)).toHaveLength(4);
  });

  it("disables every answer control when the caller cannot respond", () => {
    const markup = renderInteraction(
      promptInteraction({ respondableByCaller: false }),
    );

    expect(markup).toContain("Response unavailable");
    expect(markup.match(/type="radio"[^>]*disabled=""/g)).toHaveLength(4);
    expect(markup).toContain('disabled="">Send response</button>');
  });
});

describe("command approval details", () => {
  it("shows the task purpose and command as plain text separately from policy", () => {
    const markup = renderInteraction(
      promptInteraction({
        kind: "approval",
        content: {
          type: "approval",
          action: "process.execute",
          resource: "configured executable",
          risk: "medium",
          requestHash: "binding",
          reason: "An effect requires explicit approval",
          commandContext: {
            justification: "Check dependency versions to diagnose the build.",
            executable: "/bin/sh",
            arguments: [
              "-c",
              "cargo --version; echo '<script>not HTML</script>'",
            ],
            workingDirectory: "/work/project",
            redacted: true,
          },
        },
      }),
    );
    expect(markup).toContain("Reason — agent-provided");
    expect(markup).toContain(
      "Check dependency versions to diagnose the build.",
    );
    expect(markup).toContain("/work/project");
    expect(markup).toContain("cargo --version");
    expect(markup).not.toContain("<script>");
    expect(markup).toContain("Credential-bearing text is redacted");
    expect(markup).toContain("Show full command");
  });

  it("does not invent task intent for older targets", () => {
    const markup = renderInteraction(
      promptInteraction({
        kind: "approval",
        content: {
          type: "approval",
          action: "process.execute",
          resource: "configured executable",
          risk: null,
          requestHash: "binding",
          reason: "Approval required",
        },
      }),
    );
    expect(markup).toContain("Task-specific reason unavailable");
  });
});
