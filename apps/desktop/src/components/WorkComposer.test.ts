import { createElement, createRef } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { WorkComposer } from "./WorkComposer";

function renderComposer(attachmentsAvailable: boolean): string {
  return renderToStaticMarkup(
    createElement(WorkComposer, {
      formRef: createRef<HTMLFormElement>(),
      textareaRef: createRef<HTMLTextAreaElement>(),
      prompt: "",
      promptBytes: 0,
      promptByteLimit: 65_536,
      promptOverLimit: false,
      role: "primary",
      maxTurns: 8,
      maxTurnsLimit: 64,
      mode: "plan",
      targetLabel: "Colossus",
      canCompose: true,
      submitting: false,
      continuation: false,
      planRevision: null,
      activeWorkRunning: false,
      activeWorkNeedsInput: false,
      attachmentsAvailable,
      attachments: [],
      attachmentBusy: false,
      error: null,
      onPromptChange: vi.fn(),
      onRoleChange: vi.fn(),
      onMaxTurnsChange: vi.fn(),
      onModeChange: vi.fn(),
      onCancelPlanRevision: vi.fn(),
      onChooseAttachment: vi.fn(),
      onRemoveAttachment: vi.fn(),
      onSubmit: vi.fn(),
    }),
  );
}

function renderRevisionComposer(): string {
  return renderToStaticMarkup(
    createElement(WorkComposer, {
      formRef: createRef<HTMLFormElement>(),
      textareaRef: createRef<HTMLTextAreaElement>(),
      prompt: "",
      promptBytes: 0,
      promptByteLimit: 65_536,
      promptOverLimit: false,
      role: "primary",
      maxTurns: 8,
      maxTurnsLimit: 64,
      mode: "plan",
      targetLabel: "Colossus",
      canCompose: true,
      submitting: false,
      continuation: true,
      planRevision: { planId: "plan-1", revision: 3 },
      activeWorkRunning: false,
      activeWorkNeedsInput: false,
      attachmentsAvailable: false,
      attachments: [],
      attachmentBusy: false,
      error: null,
      onPromptChange: vi.fn(),
      onRoleChange: vi.fn(),
      onMaxTurnsChange: vi.fn(),
      onModeChange: vi.fn(),
      onCancelPlanRevision: vi.fn(),
      onChooseAttachment: vi.fn(),
      onRemoveAttachment: vi.fn(),
      onSubmit: vi.fn(),
    }),
  );
}

describe("WorkComposer capabilities", () => {
  it("hides unsupported attachment and context controls", () => {
    const markup = renderComposer(false);

    expect(markup).not.toContain("Attach a file");
    expect(markup).not.toContain("Choose workspace context");
  });

  it("renders context controls only after the capability is advertised", () => {
    const markup = renderComposer(true);

    expect(markup).toContain('aria-label="Attach a file"');
    expect(markup).toContain('aria-label="Choose workspace context"');
  });

  it("describes the durable non-mutating Plan Mode contract", () => {
    const markup = renderComposer(false);

    expect(markup).toContain("work-composer is-plan-mode");
    expect(markup).toContain(
      "Plan creates a new durable draft; implementation and external mutation are blocked.",
    );
    expect(markup).toContain("Describe the work you want Colossus to plan…");
  });

  it("makes exact-revision refinement explicit and locks the mode", () => {
    const markup = renderRevisionComposer();

    expect(markup).toContain("Revising Plan revision 3");
    expect(markup).toContain("Describe what Colossus should change");
    expect(markup).toContain("Cancel revision");
    expect(markup.match(/disabled=""/g)?.length ?? 0).toBeGreaterThanOrEqual(2);
  });
});
