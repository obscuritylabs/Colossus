import {
  IconAdjustmentsHorizontal,
  IconAt,
  IconCheck,
  IconCommand,
  IconCornerDownLeft,
  IconFolder,
  IconPaperclip,
  IconPlaylistAdd,
  IconPlugConnected,
  IconRouteAltLeft,
  IconSend2,
  IconShieldCheck,
  IconWorld,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent, RefObject } from "react";

import { desktopSlashCommandSuggestions } from "../slash-commands";
import { pluginMentionSuggestions } from "../plugins";
import type { PluginSkill } from "../plugins";
import type {
  ApprovalMode,
  ArtifactReference,
  CommandError,
  ResearchDepth,
  ResearchSourceKind,
  RunMode,
} from "../types";
import { USE_CONFIGURED_MAX_TURNS } from "../types";
import type { QueuedMessage } from "../message-queue";
import { DropdownSelect } from "./DropdownSelect";
import { NextUpQueue } from "./NextUpQueue";
import { PluginIcon } from "./PluginIcon";

const RESEARCH_DEPTH_OPTIONS = [
  { value: "quick", label: "Quick" },
  { value: "standard", label: "Standard" },
  { value: "deep", label: "Deep" },
] as const;

const RESEARCH_SOURCE_OPTIONS = [
  {
    value: "repo",
    label: "This Workspace",
    description: "Search across your workspace",
    Icon: IconFolder,
  },
  {
    value: "web",
    label: "Web",
    description: "Search the public web",
    Icon: IconWorld,
  },
  {
    value: "mcp",
    label: "Connections",
    description: "Search your connected apps",
    Icon: IconPlugConnected,
  },
] as const;

interface WorkComposerProps {
  pluginSkills?: readonly PluginSkill[] | null;
  pluginSelections?: readonly string[];
  onRemovePluginSkill?: (id: string) => void;
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
  researchDepth: ResearchDepth;
  researchSources: readonly ResearchSourceKind[];
  researchAvailable: boolean;
  approvalMode: ApprovalMode;
  approvalModeVisible: boolean;
  approvalModeAvailable: boolean;
  approvalModeChanging: boolean;
  targetLabel: string;
  canCompose: boolean;
  submitting: boolean;
  continuation: boolean;
  planRevision: { planId: string; revision: number } | null;
  queueing: boolean;
  activeWorkRunning: boolean;
  activeWorkNeedsInput: boolean;
  activeWorkRedirectable: boolean;
  queuedMessages: readonly QueuedMessage[];
  attachmentsAvailable: boolean;
  attachments: readonly ArtifactReference[];
  attachmentBusy: boolean;
  error: CommandError | null;
  onPromptChange: (prompt: string) => void;
  onRoleChange: (role: string) => void;
  onMaxTurnsChange: (maxTurns: number) => void;
  onModeChange: (mode: RunMode) => void;
  onResearchDepthChange: (depth: ResearchDepth) => void;
  onResearchSourcesChange: (sources: ResearchSourceKind[]) => void;
  onApprovalModeChange: (mode: ApprovalMode) => void;
  onCancelPlanRevision: () => void;
  onChooseAttachment: () => void;
  onRemoveAttachment: (artifactId: string) => void;
  onEditQueuedMessage: (messageId: string, prompt: string) => void;
  onDeleteQueuedMessage: (messageId: string) => void;
  onRetryQueuedMessage: (messageId: string) => void;
  onRedirect: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}

export function WorkComposer({
  pluginSkills = null,
  pluginSelections = [],
  onRemovePluginSkill,
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
  researchDepth,
  researchSources,
  researchAvailable,
  approvalMode,
  approvalModeVisible,
  approvalModeAvailable,
  approvalModeChanging,
  targetLabel,
  canCompose,
  submitting,
  continuation,
  planRevision,
  queueing,
  activeWorkRunning,
  activeWorkNeedsInput,
  activeWorkRedirectable,
  queuedMessages,
  attachmentsAvailable,
  attachments,
  attachmentBusy,
  error,
  onPromptChange,
  onRoleChange,
  onMaxTurnsChange,
  onModeChange,
  onResearchDepthChange,
  onResearchSourcesChange,
  onApprovalModeChange,
  onCancelPlanRevision,
  onChooseAttachment,
  onRemoveAttachment,
  onEditQueuedMessage,
  onDeleteQueuedMessage,
  onRetryQueuedMessage,
  onRedirect,
  onSubmit,
}: WorkComposerProps) {
  const [selectedSlashCommand, setSelectedSlashCommand] = useState<
    string | null
  >(null);
  const [dismissedSlashDraft, setDismissedSlashDraft] = useState<string | null>(
    null,
  );
  const roleMissing = role.trim().length === 0;
  const slashCommandDraft = prompt.trimStart().startsWith("/");
  const mentioningSkill = prompt.trimStart().startsWith("@");
  const slashCommandSuggestions = mentioningSkill
    ? pluginMentionSuggestions(prompt, pluginSkills ?? [])
    : desktopSlashCommandSuggestions(prompt);
  const staleSelections =
    pluginSkills === null
      ? []
      : pluginSelections.filter(
          (id) => !pluginSkills.some((skill) => skill.id === id),
        );
  const slashMenuOpen =
    slashCommandSuggestions.length > 0 && dismissedSlashDraft !== prompt;
  const selectedSlashIndex = slashCommandSuggestions.findIndex(
    ({ command }) => command === selectedSlashCommand,
  );
  const activeSlashIndex =
    slashMenuOpen && slashCommandSuggestions.length > 0
      ? Math.max(0, selectedSlashIndex)
      : -1;
  const slashOptionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const researchSourceSummary = researchSources
    .map((source) =>
      source === "repo"
        ? "This Workspace"
        : source === "web"
          ? "Web"
          : "Connections",
    )
    .join(", ");

  useEffect(() => {
    if (activeSlashIndex < 0) {
      return;
    }
    slashOptionRefs.current[activeSlashIndex]?.scrollIntoView({
      block: "nearest",
    });
  }, [activeSlashIndex, prompt, slashMenuOpen]);

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (slashMenuOpen && event.key === "Escape") {
      event.preventDefault();
      setDismissedSlashDraft(prompt);
      setSelectedSlashCommand(null);
      return;
    }
    if (
      slashMenuOpen &&
      (event.key === "ArrowDown" || event.key === "ArrowUp")
    ) {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      const nextIndex =
        (activeSlashIndex + direction + slashCommandSuggestions.length) %
        slashCommandSuggestions.length;
      setSelectedSlashCommand(
        slashCommandSuggestions[nextIndex]?.command ?? null,
      );
      return;
    }
    const completion =
      slashCommandSuggestions[activeSlashIndex < 0 ? 0 : activeSlashIndex];
    if (
      slashMenuOpen &&
      completion !== undefined &&
      (event.key === "Tab" ||
        (event.key === "ArrowRight" && selectedSlashIndex >= 0))
    ) {
      event.preventDefault();
      setSelectedSlashCommand(completion.command);
      setDismissedSlashDraft(null);
      onPromptChange(completion.command);
      return;
    }
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      if (
        slashMenuOpen &&
        completion !== undefined &&
        prompt.trim().toLowerCase() !== completion.command
      ) {
        setSelectedSlashCommand(completion.command);
        setDismissedSlashDraft(null);
        onPromptChange(completion.command);
        return;
      }
      formRef.current?.requestSubmit();
    }
  }

  return (
    <form
      ref={formRef}
      className={`work-composer${mode === "plan" ? " is-plan-mode" : ""}${mode === "research" ? " is-research-mode" : ""}`}
      id="work-composer"
      aria-label="Send a prompt"
      onSubmit={onSubmit}
    >
      {pluginSelections.length > 0 && (
        <div className="plugin-selections" aria-label="Conversation skills">
          <span>Conversation skills:</span>
          {pluginSelections.map((id) => (
            <button
              className="button secondary compact"
              type="button"
              key={id}
              onClick={() => onRemovePluginSkill?.(id)}
              aria-label={`Remove ${id}`}
            >
              <PluginIcon
                name={id.split("/")[0] ?? id}
                icon={
                  pluginSkills?.find((skill) => skill.id === id)?.icon_data_url
                }
                size="small"
              />
              {id} ×
            </button>
          ))}
        </div>
      )}
      {staleSelections.length > 0 && (
        <p role="alert" className="inline-error">
          Unavailable conversation skills: {staleSelections.join(", ")}. Remove
          them or re-enable their plugin before sending.
        </p>
      )}
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
        {approvalModeVisible ? (
          <label className={`approval-mode-control mode-${approvalMode}`}>
            <IconShieldCheck size={15} stroke={1.7} aria-hidden="true" />
            <span className="sr-only">Permission mode</span>
            <DropdownSelect
              aria-label="Permission mode"
              value={approvalMode}
              disabled={
                !approvalModeAvailable || approvalModeChanging || submitting
              }
              onChange={(event) =>
                onApprovalModeChange(event.target.value as ApprovalMode)
              }
            >
              <option value="deny">Deny</option>
              <option value="ask">Ask</option>
              <option value="risk_auto">Risk auto</option>
              <option value="full_access">Full access</option>
            </DropdownSelect>
          </label>
        ) : null}
        <details
          className="run-controls"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.currentTarget.removeAttribute("open");
              event.currentTarget.querySelector("summary")?.focus();
            }
          }}
        >
          <summary
            className={roleMissing ? "run-controls-invalid" : undefined}
            aria-label={
              mode === "research"
                ? `Research controls, sources ${researchSourceSummary || "none"}`
                : roleMissing
                  ? "Advanced run controls, role required"
                  : "Advanced run controls"
            }
          >
            <IconAdjustmentsHorizontal
              size={16}
              stroke={1.7}
              aria-hidden="true"
            />
            {mode === "research"
              ? `Sources: ${researchSourceSummary || "None"}`
              : roleMissing
                ? "Role required"
                : "Run controls"}
          </summary>
          <div
            className={`run-controls-popover${mode === "research" ? " is-research" : ""}`}
          >
            {mode === "research" ? (
              <>
                <header className="research-settings-header">
                  <h3>Research settings</h3>
                  <button
                    className="research-settings-close"
                    type="button"
                    aria-label="Close research settings"
                    onClick={(event) => {
                      const controls = event.currentTarget.closest("details");
                      controls?.removeAttribute("open");
                      controls?.querySelector("summary")?.focus();
                    }}
                  >
                    <IconX size={17} stroke={1.8} aria-hidden="true" />
                  </button>
                </header>
                <fieldset className="research-depth-controls">
                  <legend>Research depth</legend>
                  <div className="research-depth-options">
                    {RESEARCH_DEPTH_OPTIONS.map((option) => (
                      <label
                        className="research-depth-option"
                        key={option.value}
                      >
                        <input
                          type="radio"
                          name="research-depth"
                          value={option.value}
                          checked={researchDepth === option.value}
                          disabled={submitting}
                          onChange={() => onResearchDepthChange(option.value)}
                        />
                        <span>{option.label}</span>
                      </label>
                    ))}
                  </div>
                </fieldset>
                <fieldset className="research-source-controls">
                  <legend>Evidence sources</legend>
                  <div className="research-source-options">
                    {RESEARCH_SOURCE_OPTIONS.map((option) => {
                      const selected = researchSources.includes(option.value);
                      return (
                        <label
                          className={`research-source-option${selected ? " is-selected" : ""}`}
                          key={option.value}
                        >
                          <input
                            type="checkbox"
                            checked={selected}
                            disabled={submitting}
                            onChange={(event) => {
                              const next = event.target.checked
                                ? [...researchSources, option.value]
                                : researchSources.filter(
                                    (item) => item !== option.value,
                                  );
                              onResearchSourcesChange(next);
                            }}
                          />
                          <span
                            className="research-source-icon"
                            aria-hidden="true"
                          >
                            <option.Icon size={19} stroke={1.7} />
                          </span>
                          <span className="research-source-copy">
                            <strong>{option.label}</strong>
                            <small>{option.description}</small>
                          </span>
                          <span
                            className="research-source-checkbox"
                            aria-hidden="true"
                          >
                            {selected ? (
                              <IconCheck size={14} stroke={2.4} />
                            ) : null}
                          </span>
                        </label>
                      );
                    })}
                  </div>
                </fieldset>
              </>
            ) : (
              <>
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
                    value={
                      maxTurns === USE_CONFIGURED_MAX_TURNS ? "" : maxTurns
                    }
                    placeholder="Server default"
                    min={1}
                    max={maxTurnsLimit}
                    aria-describedby="max-turns-default-hint"
                    disabled={submitting}
                    onChange={(event) =>
                      onMaxTurnsChange(Number(event.target.value))
                    }
                  />
                  <span
                    className="run-control-hint"
                    id="max-turns-default-hint"
                  >
                    Leave blank to use the server default.
                  </span>
                </label>
              </>
            )}
          </div>
        </details>
      </div>
      <NextUpQueue
        messages={queuedMessages}
        onEdit={onEditQueuedMessage}
        onDelete={onDeleteQueuedMessage}
        onRetry={onRetryQueuedMessage}
      />
      {slashMenuOpen ? (
        <div className="slash-command-menu">
          <header className="slash-command-header">
            <span className="slash-command-title">
              <span className="slash-command-icon" aria-hidden="true">
                <IconCommand size={16} stroke={1.9} />
              </span>
              <span>
                <strong>
                  {mentioningSkill
                    ? slashCommandSuggestions[0]?.group === "Plugin"
                      ? "Plugins"
                      : "Plugin skills"
                    : "Commands"}
                </strong>
                <small>
                  {mentioningSkill
                    ? slashCommandSuggestions[0]?.group === "Plugin"
                      ? "Choose a plugin to explore its skills"
                      : "Use for this message only"
                    : "Run a local Desktop action"}
                </small>
              </span>
            </span>
            <span className="slash-command-count" aria-live="polite">
              {slashCommandSuggestions.length}
            </span>
          </header>
          <div
            className="slash-command-options"
            id="desktop-slash-command-menu"
            role="listbox"
            aria-label={mentioningSkill ? "Plugin skills" : "Slash commands"}
          >
            {slashCommandSuggestions.map((suggestion, index) => {
              const selected = index === activeSlashIndex;
              return (
                <button
                  ref={(node) => {
                    slashOptionRefs.current[index] = node;
                  }}
                  id={`desktop-slash-command-${index}`}
                  key={suggestion.command}
                  title={suggestion.command.trim()}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  className={`${selected ? "is-selected" : ""}${"plugin" in suggestion ? " is-plugin-suggestion" : ""}`}
                  onMouseDown={(event) => event.preventDefault()}
                  onMouseEnter={() =>
                    setSelectedSlashCommand(suggestion.command)
                  }
                  onClick={() => {
                    setSelectedSlashCommand(suggestion.command);
                    setDismissedSlashDraft(null);
                    onPromptChange(suggestion.command);
                    textareaRef.current?.focus();
                  }}
                >
                  {"plugin" in suggestion && (
                    <PluginIcon
                      name={suggestion.plugin}
                      icon={suggestion.icon}
                    />
                  )}
                  <span className="slash-command-copy">
                    <strong>
                      {"label" in suggestion
                        ? suggestion.label
                        : suggestion.command}
                    </strong>
                    <small>{suggestion.description}</small>
                  </span>
                  <span className="slash-command-trailing">
                    <em>{suggestion.group}</em>
                    <IconCornerDownLeft
                      className="slash-command-enter"
                      size={15}
                      stroke={1.8}
                      aria-hidden="true"
                    />
                  </span>
                </button>
              );
            })}
          </div>
          <footer className="slash-command-footer">
            <span className="slash-command-local">
              {mentioningSkill ? (
                <IconAt size={14} stroke={1.8} aria-hidden="true" />
              ) : (
                <IconShieldCheck size={14} stroke={1.8} aria-hidden="true" />
              )}
              {mentioningSkill ? "For this message" : "Local to Desktop"}
            </span>
            <span className="slash-command-keys" aria-hidden="true">
              <span>
                <kbd>↑</kbd>
                <kbd>↓</kbd> Navigate
              </span>
              <span>
                <kbd>Tab</kbd> Complete
              </span>
              <span>
                <kbd>Esc</kbd> Close
              </span>
            </span>
          </footer>
        </div>
      ) : null}
      <textarea
        ref={textareaRef}
        value={prompt}
        rows={3}
        maxLength={65_536}
        placeholder={
          activeWorkNeedsInput
            ? "Add a follow-up to Next up, or answer the request above…"
            : activeWorkRunning
              ? "Add a follow-up while Colossus keeps working…"
              : queueing
                ? "Add another message to Next up…"
                : planRevision !== null
                  ? "Describe what Colossus should change in this Plan…"
                  : continuation
                    ? "Continue this thread…"
                    : mode === "plan"
                      ? "Describe the work you want Colossus to plan…"
                      : mode === "research"
                        ? "Ask a source-backed question…"
                        : "Ask Colossus to work on something…"
        }
        aria-label="Prompt"
        aria-invalid={promptOverLimit}
        aria-autocomplete="list"
        aria-controls={slashMenuOpen ? "desktop-slash-command-menu" : undefined}
        aria-activedescendant={
          slashMenuOpen && activeSlashIndex >= 0
            ? `desktop-slash-command-${activeSlashIndex}`
            : undefined
        }
        aria-describedby={
          promptOverLimit ? "prompt-byte-limit-error" : undefined
        }
        disabled={!canCompose || submitting}
        onKeyDown={handleKeyDown}
        onBlur={(event) => {
          const nextTarget = event.relatedTarget as Node | null;
          if (
            nextTarget === null ||
            !event.currentTarget
              .closest("form")
              ?.querySelector(".slash-command-menu")
              ?.contains(nextTarget)
          ) {
            setDismissedSlashDraft(prompt);
            setSelectedSlashCommand(null);
          }
        }}
        onChange={(event) => {
          setSelectedSlashCommand(null);
          setDismissedSlashDraft(null);
          onPromptChange(event.target.value);
        }}
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
              title="Attach a PNG, JPEG, WebP, UTF-8 text, or source file"
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
          <label>
            <input
              type="radio"
              name="mode"
              value="research"
              checked={mode === "research"}
              disabled={
                submitting || planRevision !== null || !researchAvailable
              }
              onChange={() => onModeChange("research")}
            />
            <span
              title={
                researchAvailable
                  ? undefined
                  : "Research is unavailable for this target"
              }
            >
              Research
            </span>
          </label>
        </fieldset>
        {activeWorkRunning && !slashCommandDraft ? (
          <button
            className="redirect-button"
            type="button"
            aria-label="Redirect current response"
            title="Stop the current response and send this guidance next"
            disabled={
              !canCompose ||
              !activeWorkRedirectable ||
              prompt.trim().length === 0 ||
              promptOverLimit ||
              roleMissing
            }
            onClick={onRedirect}
          >
            <IconRouteAltLeft size={16} stroke={1.9} aria-hidden="true" />
            Redirect
          </button>
        ) : null}
        <button
          className={`send-button${queueing && !slashCommandDraft ? " is-queue" : ""}`}
          type="submit"
          aria-label={
            submitting
              ? "Sending prompt"
              : slashCommandDraft
                ? "Run command"
                : queueing
                  ? "Add message to Next up"
                  : "Send prompt"
          }
          disabled={
            !canCompose ||
            prompt.trim().length === 0 ||
            promptOverLimit ||
            (!slashCommandDraft &&
              (roleMissing ||
                (mode === "research" && researchSources.length === 0)))
          }
        >
          {submitting ? (
            <span className="spinner" aria-hidden="true" />
          ) : queueing && !slashCommandDraft ? (
            <>
              <IconPlaylistAdd size={18} stroke={1.9} aria-hidden="true" />
              <span>Queue</span>
            </>
          ) : (
            <IconSend2 size={19} stroke={2} aria-hidden="true" />
          )}
        </button>
      </div>
      <div className="composer-meta">
        <span>
          {slashCommandDraft
            ? "Slash commands run locally in Desktop and are never sent to the model."
            : mode === "research" && researchSources.length === 0
              ? "Select at least one evidence source before starting Research."
              : activeWorkRunning
                ? activeWorkNeedsInput
                  ? "Queued messages wait until the required response is resolved. Redirect stops this response and sends your guidance next."
                  : "Enter adds to Next up. Redirect stops this response and sends your guidance next."
                : queueing
                  ? "New messages join Next up. Resolve or remove a failed item to continue in order."
                  : mode === "plan"
                    ? planRevision === null
                      ? "Plan creates a new durable draft; implementation and external mutation are blocked."
                      : "This prompt revises the selected draft; implementation and external mutation remain blocked."
                    : mode === "research"
                      ? "Research gathers released evidence from the selected sources and returns a citation-backed report."
                      : !approvalModeVisible
                        ? "Effects remain policy-bound and may require approval."
                        : approvalMode === "deny"
                          ? "Approval-required effects are denied. Policy and sandbox boundaries remain active."
                          : approvalMode === "ask"
                            ? "Approval-required effects pause and ask before continuing."
                            : approvalMode === "risk_auto"
                              ? "Eligible low-risk approvals may proceed automatically; other effects ask."
                              : "Approval obligations proceed without asking; policy and sandbox boundaries remain active."}
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
