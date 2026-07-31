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
