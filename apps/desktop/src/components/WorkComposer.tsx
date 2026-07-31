import {
  IconAdjustmentsHorizontal,
  IconAt,
  IconFolder,
  IconPaperclip,
  IconSend2,
} from "@tabler/icons-react";
import type { FormEvent, KeyboardEvent, RefObject } from "react";

import type { ArtifactReference, CommandError, RunMode } from "../types";

interface WorkComposerProps {
  formRef: RefObject<HTMLFormElement | null>;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  prompt: string;
  promptBytes: number;
  promptByteLimit: number;
  promptOverLimit: boolean;
  role: string;
  maxTurns: number;
  maxTurnsLimit: number;
  mode: RunMode;
  targetLabel: string;
  canCompose: boolean;
  submitting: boolean;
  continuation: boolean;
  planRevision: { planId: string; revision: number } | null;
  activeWorkRunning: boolean;
  activeWorkNeedsInput: boolean;
  attachmentsAvailable: boolean;
  attachments: readonly ArtifactReference[];
  attachmentBusy: boolean;
  error: CommandError | null;
  onPromptChange: (prompt: string) => void;
  onRoleChange: (role: string) => void;
  onMaxTurnsChange: (maxTurns: number) => void;
  onModeChange: (mode: RunMode) => void;
  onCancelPlanRevision: () => void;
  onChooseAttachment: () => void;
  onRemoveAttachment: (artifactId: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}

export function WorkComposer({
  formRef,
  textareaRef,
  prompt,
  promptBytes,
  promptByteLimit,
  promptOverLimit,
  role,
  maxTurns,
  maxTurnsLimit,
  mode,
  targetLabel,
  canCompose,
  submitting,
  continuation,
  planRevision,
  activeWorkRunning,
  activeWorkNeedsInput,
  attachmentsAvailable,
  attachments,
  attachmentBusy,
  error,
  onPromptChange,
  onRoleChange,
  onMaxTurnsChange,
  onModeChange,
  onCancelPlanRevision,
  onChooseAttachment,
  onRemoveAttachment,
  onSubmit,
}: WorkComposerProps) {
  const roleMissing = role.trim().length === 0;

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      formRef.current?.requestSubmit();
    }
  }

  return (
    <form
      ref={formRef}
      className={`work-composer${mode === "plan" ? " is-plan-mode" : ""}`}
      id="work-composer"
      aria-label="Send a prompt"
      onSubmit={onSubmit}
    >
      {planRevision === null ? null : (
        <div className="composer-plan-revision" role="status">
          <div>
            <strong>Revising Plan revision {planRevision.revision}</strong>
            <span>
              Your next prompt will update this exact draft in the current
              session.
            </span>
          </div>
          <button
            type="button"
            disabled={submitting}
            onClick={onCancelPlanRevision}
          >
            Cancel revision
          </button>
        </div>
      )}
      <div className="composer-target-row">
        <span className="composer-target">
          <IconAt size={15} stroke={1.8} aria-hidden="true" />
          {targetLabel}
        </span>
        <details className="run-controls">
          <summary
            className={roleMissing ? "run-controls-invalid" : undefined}
            aria-label={
              roleMissing
                ? "Advanced run controls, role required"
                : "Advanced run controls"
            }
          >
            <IconAdjustmentsHorizontal
              size={16}
              stroke={1.7}
              aria-hidden="true"
            />
            {roleMissing ? "Role required" : "Run controls"}
          </summary>
          <div className="run-controls-popover">
            <label>
              <span>Role</span>
              <input
                value={role}
                maxLength={64}
                required
                aria-invalid={roleMissing}
                aria-describedby={
                  roleMissing ? "role-required-error" : undefined
                }
                disabled={submitting}
                onChange={(event) => onRoleChange(event.target.value)}
              />
              {roleMissing ? (
                <span
                  className="run-control-error"
                  id="role-required-error"
                  role="alert"
                >
                  Enter the enrolled agent role used for this run.
                </span>
              ) : null}
            </label>
            <label>
              <span>Maximum turns</span>
              <input
                type="number"
                value={maxTurns}
                min={1}
                max={maxTurnsLimit}
                disabled={submitting}
                onChange={(event) =>
                  onMaxTurnsChange(Number(event.target.value))
                }
              />
            </label>
          </div>
        </details>
      </div>
      <textarea
        ref={textareaRef}
        value={prompt}
        rows={3}
        maxLength={65_536}
        placeholder={
          activeWorkNeedsInput
            ? "Respond to the request above before sending another prompt."
            : activeWorkRunning
              ? "This run is working. Cancel it or start new work to send another prompt."
              : planRevision !== null
                ? "Describe what Colossus should change in this Plan…"
                : continuation
                  ? "Continue this thread…"
                  : mode === "plan"
                    ? "Describe the work you want Colossus to plan…"
                    : "Ask Colossus to work on something…"
        }
        aria-label="Prompt"
        aria-invalid={promptOverLimit}
        aria-describedby={
          promptOverLimit ? "prompt-byte-limit-error" : undefined
        }
        disabled={!canCompose}
        onKeyDown={handleKeyDown}
        onChange={(event) => onPromptChange(event.target.value)}
      />
      {attachments.length > 0 ? (
        <div className="composer-attachments" aria-label="Run attachments">
          {attachments.map((attachment) => (
            <span className="artifact-chip" key={attachment.artifactId}>
              {attachment.fileName}
              <button
                type="button"
                aria-label={`Remove ${attachment.fileName}`}
                onClick={() => onRemoveAttachment(attachment.artifactId)}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      ) : null}
      <div className="composer-action-row">
        {attachmentsAvailable ? (
          <div className="composer-context-actions">
            <button
              className="icon-button"
              type="button"
              disabled={
                !canCompose || attachmentBusy || attachments.length >= 16
              }
              aria-label="Attach a file"
              title="Attach a UTF-8 text or source file"
              onClick={onChooseAttachment}
            >
              <IconPaperclip size={19} stroke={1.7} aria-hidden="true" />
            </button>
            <button
              className="icon-button"
              type="button"
              disabled
              aria-label="Choose workspace context"
              title="Workspace context selection is not available in this Desktop version"
            >
              <IconFolder size={19} stroke={1.7} aria-hidden="true" />
            </button>
          </div>
        ) : null}
        <fieldset className="mode-switch">
          <legend className="sr-only">Run mode</legend>
          <label>
            <input
              type="radio"
              name="mode"
              value="plan"
              checked={mode === "plan"}
              disabled={submitting || planRevision !== null}
              onChange={() => onModeChange("plan")}
            />
            <span>Plan</span>
          </label>
          <label>
            <input
              type="radio"
              name="mode"
              value="execute"
              checked={mode === "execute"}
              disabled={submitting || planRevision !== null}
              onChange={() => onModeChange("execute")}
            />
            <span>Execute</span>
          </label>
        </fieldset>
        <button
          className="send-button"
          type="submit"
          aria-label={submitting ? "Sending prompt" : "Send prompt"}
          disabled={
            !canCompose ||
            prompt.trim().length === 0 ||
            promptOverLimit ||
            roleMissing
          }
        >
          {submitting ? (
            <span className="spinner" aria-hidden="true" />
          ) : (
            <IconSend2 size={19} stroke={2} aria-hidden="true" />
          )}
        </button>
      </div>
      <div className="composer-meta">
        <span>
          {mode === "plan"
            ? planRevision === null
              ? "Plan creates a new durable draft; implementation and external mutation are blocked."
              : "This prompt revises the selected draft; implementation and external mutation remain blocked."
            : "Effects remain policy-bound and may require approval."}
        </span>
        <span className={promptOverLimit ? "counter-over-limit" : undefined}>
          {promptBytes.toLocaleString()} / {promptByteLimit.toLocaleString()}{" "}
          bytes
        </span>
      </div>
      {promptOverLimit ? (
        <p
          className="prompt-limit-error"
          id="prompt-byte-limit-error"
          role="alert"
        >
          Prompt is too large. Shorten it to {promptByteLimit.toLocaleString()}{" "}
          UTF-8 bytes.
        </p>
      ) : null}
      {error !== null ? (
        <div className="composer-error" role="alert">
          <span>{error.message}</span>
          {error.outcomeUnknown ? (
            <strong>Outcome unknown — do not retry automatically.</strong>
          ) : null}
          {error.retryable && !error.outcomeUnknown ? (
            <span>Retrying will use the same request key.</span>
          ) : null}
        </div>
      ) : null}
    </form>
  );
}
