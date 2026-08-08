import { createElement, createRef } from "react";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { WorkComposer } from "./WorkComposer";

type WorkComposerProps = ComponentProps<typeof WorkComposer>;

function renderComposer(
  attachmentsAvailable: boolean,
  overrides: Partial<WorkComposerProps> = {},
): string {
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
      approvalMode: "ask",
      approvalModeVisible: true,
      approvalModeAvailable: true,
      approvalModeChanging: false,
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
      onApprovalModeChange: vi.fn(),
      onCancelPlanRevision: vi.fn(),
      onChooseAttachment: vi.fn(),
      onRemoveAttachment: vi.fn(),
      onSubmit: vi.fn(),
      ...overrides,
    }),
  );
}

function renderRevisionComposer(): string {
  return renderComposer(false, {
    continuation: true,
    planRevision: { planId: "plan-1", revision: 3 },
  });
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

describe("WorkComposer permission mode", () => {
  it("shows every native permission choice and selects the current mode", () => {
    const markup = renderComposer(false, {
      mode: "execute",
      approvalMode: "risk_auto",
    });

    expect(markup).toContain('aria-label="Permission mode"');
    expect(markup).toContain('<option value="deny">Deny</option>');
    expect(markup).toContain('<option value="ask">Ask</option>');
    expect(markup).toContain(
      '<option value="risk_auto" selected="">Risk auto</option>',
    );
    expect(markup).toContain(
      '<option value="full_access">Full access</option>',
    );
  });

  it("disables switching when Managed Local cannot accept a live change", () => {
    const markup = renderComposer(false, { approvalModeAvailable: false });
    const selectStart = markup.indexOf('aria-label="Permission mode"');

    expect(selectStart).toBeGreaterThan(-1);
    expect(
      markup.slice(selectStart, markup.indexOf(">", selectStart)),
    ).toContain("disabled");
  });

  it("does not present Managed Local permissions for an External target", () => {
    expect(renderComposer(false, { approvalModeVisible: false })).not.toContain(
      'aria-label="Permission mode"',
    );
  });
});
