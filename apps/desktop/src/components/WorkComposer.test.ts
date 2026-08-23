import { createElement, createRef } from "react";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { USE_CONFIGURED_MAX_TURNS } from "../types";
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
      researchDepth: "standard",
      researchSources: ["repo", "web", "mcp"],
      researchAvailable: true,
      approvalMode: "ask",
      approvalModeVisible: true,
      approvalModeAvailable: true,
      approvalModeChanging: false,
      targetLabel: "Colossus",
      canCompose: true,
      submitting: false,
      continuation: false,
      planRevision: null,
      queueing: false,
      activeWorkRunning: false,
      activeWorkNeedsInput: false,
      activeWorkRedirectable: false,
      queuedMessages: [],
      attachmentsAvailable,
      attachments: [],
      attachmentBusy: false,
      error: null,
      onPromptChange: vi.fn(),
      onRoleChange: vi.fn(),
      onMaxTurnsChange: vi.fn(),
      onModeChange: vi.fn(),
      onResearchDepthChange: vi.fn(),
      onResearchSourcesChange: vi.fn(),
      onApprovalModeChange: vi.fn(),
      onCancelPlanRevision: vi.fn(),
      onChooseAttachment: vi.fn(),
      onRemoveAttachment: vi.fn(),
      onEditQueuedMessage: vi.fn(),
      onDeleteQueuedMessage: vi.fn(),
      onRetryQueuedMessage: vi.fn(),
      onRedirect: vi.fn(),
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
  it("renders the dedicated Research composer controls", () => {
    const markup = renderComposer(false, {
      mode: "research",
      researchDepth: "standard",
      researchSources: ["repo", "web", "mcp"],
    });

    expect(markup).toContain("Ask a source-backed question");
    expect(markup).toContain("Research depth");
    expect(markup).toContain("This Workspace");
    expect(markup).toContain("Connections");
    expect(markup).toContain("Research settings");
    expect(markup).toContain('aria-label="Close research settings"');
    expect(markup).toContain('name="research-depth"');
    expect(markup).toContain("Search across your workspace");
    expect(markup).toContain("Search the public web");
    expect(markup).toContain("Search your connected apps");
    expect(markup).toContain("Sources: This Workspace, Web, Connections");
    expect(markup).toContain(
      'aria-label="Research controls, sources This Workspace, Web, Connections"',
    );
  });

  it("requires at least one Research evidence source", () => {
    const markup = renderComposer(false, {
      mode: "research",
      researchSources: [],
      prompt: "What changed?",
      promptBytes: 13,
    });

    expect(markup).toContain(
      "Select at least one evidence source before starting Research.",
    );
    const sendStart = markup.indexOf('aria-label="Send prompt"');
    expect(markup.slice(sendStart, markup.indexOf(">", sendStart))).toContain(
      "disabled",
    );
  });

  it("leaves the turn override blank when the server default is selected", () => {
    const markup = renderComposer(false, {
      maxTurns: USE_CONFIGURED_MAX_TURNS,
    });

    expect(markup).toContain('placeholder="Server default"');
    expect(markup).toContain('value=""');
    expect(markup).toContain("Leave blank to use the server default.");
  });

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
  it("shows the app-owned permission control with the current mode", () => {
    const markup = renderComposer(false, {
      mode: "execute",
      researchDepth: "standard",
      researchSources: ["repo", "web", "mcp"],
      approvalMode: "risk_auto",
    });

    expect(markup).toContain('aria-label="Permission mode"');
    expect(markup).toContain('role="combobox"');
    expect(markup).toContain('<span class="app-select-value">Risk auto</span>');
    expect(markup).not.toContain("<select");
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

describe("WorkComposer follow-ups", () => {
  it("keeps the composer available and names queue and redirect actions while work runs", () => {
    const markup = renderComposer(false, {
      mode: "execute",
      queueing: true,
      activeWorkRunning: true,
      activeWorkRedirectable: true,
      prompt: "Check the Windows path too",
      promptBytes: 26,
    });

    expect(markup).toContain("Add a follow-up while Colossus keeps working");
    expect(markup).toContain('aria-label="Add message to Next up"');
    expect(markup).toContain('aria-label="Redirect current response"');
    expect(markup).toContain("Enter adds to Next up");
  });

  it("renders editable and deletable queued messages", () => {
    const markup = renderComposer(false, {
      activeWorkRunning: true,
      queueing: true,
      activeWorkRedirectable: true,
      queuedMessages: [
        {
          id: "queued-1",
          idempotencyKey: "key-1",
          targetId: "target-1",
          sessionId: "session-1",
          prompt: "Also inspect the Windows path",
          role: "primary",
          mode: "execute",
          researchDepth: "standard",
          researchSources: ["repo", "web", "mcp"],
          maxTurns: 0,
          attachments: [],
          createdAt: "2026-08-14T12:00:00Z",
          state: "pending",
          error: null,
        },
      ],
    });

    expect(markup).toContain('aria-label="Next up"');
    expect(markup).toContain("Also inspect the Windows path");
    expect(markup).toContain('aria-label="Edit queued message"');
    expect(markup).toContain('aria-label="Delete queued message"');
  });
});
