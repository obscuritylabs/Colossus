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
});
