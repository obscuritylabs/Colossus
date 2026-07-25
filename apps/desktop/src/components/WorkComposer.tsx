import {
  IconAdjustmentsHorizontal,
  IconAt,
  IconFolder,
  IconPaperclip,
  IconSend2,
} from "@tabler/icons-react";
import type { FormEvent, KeyboardEvent, RefObject } from "react";

import type { CommandError, RunMode } from "../types";

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
  activeWorkRunning: boolean;
  activeWorkNeedsInput: boolean;
  attachmentsAvailable: boolean;
  error: CommandError | null;
  onPromptChange: (prompt: string) => void;
  onRoleChange: (role: string) => void;
  onMaxTurnsChange: (maxTurns: number) => void;
  onModeChange: (mode: RunMode) => void;
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
  activeWorkRunning,
  activeWorkNeedsInput,
  attachmentsAvailable,
  error,
  onPromptChange,
  onRoleChange,
  onMaxTurnsChange,
  onModeChange,
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
      className="work-composer"
      id="work-composer"
      aria-label="Send a prompt"
      onSubmit={onSubmit}
    >
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
              : continuation
                ? "Continue this thread…"
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
      <div className="composer-action-row">
        {attachmentsAvailable ? (
          <div className="composer-context-actions">
            <button
              className="icon-button"
              type="button"
              disabled
              aria-label="Attach a file"
              title="File attachment is not available in this Desktop version"
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
              disabled={submitting}
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
              disabled={submitting}
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
            ? "Plan mode blocks implementation and external changes."
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
