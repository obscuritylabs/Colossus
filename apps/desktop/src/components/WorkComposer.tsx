import {
  IconAdjustmentsHorizontal,
  IconAt,
  IconCommand,
  IconCornerDownLeft,
  IconFileText,
  IconPaperclip,
  IconPlaylistAdd,
  IconPlayerStopFilled,
  IconRouteAltLeft,
  IconShieldCheck,
} from "@tabler/icons-react";
import {
  GoalControls,
  DEFAULT_GOAL_ITERATIONS,
  validGoalIterations,
} from "@colossus/ui/components/GoalControls";
import { ConversationComposerFrame } from "@colossus/ui/conversation";
import {
  ResearchControls,
  RESEARCH_SOURCE_OPTIONS,
} from "@colossus/ui/components/ResearchControls";
import "@colossus/ui/styles/research-controls.css";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent, ReactNode, RefObject } from "react";

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
import type { ComposerModelContext } from "../composer-model";
import {
  ComposerInput,
  ComposerModeSwitch,
  ComposerSendButton,
  isComposerSendKey,
} from "@colossus/ui";
import type { ComposerEditIntent } from "../composer-paste";
import type { DictationController, DictationSnapshot } from "../dictation";
import { DictationControl } from "./DictationControl";
import { DictationRecordingBar } from "./DictationRecordingBar";

const ComposerModelChip = lazy(() =>
  import("./ComposerModelChip").then((module) => ({
    default: module.ComposerModelChip,
  })),
);

interface WorkComposerProps {
  onOpenDictationSettings?: (() => void) | undefined;
  dictation?:
    { controller: DictationController; state: DictationSnapshot } | undefined;
  contextActions?: ReactNode;
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
  goalAvailable?: boolean;
  goalMaxIterations?: number;
  onGoalMaxIterationsChange?: (value: number) => void;
  approvalMode: ApprovalMode;
  approvalModeVisible: boolean;
  approvalModeAvailable: boolean;
  approvalModeChanging: boolean;
  modelContext?: ComposerModelContext;
  onOpenModelSettings?: () => void;
  canCompose: boolean;
  submitting: boolean;
  continuation: boolean;
  planRevision: { planId: string; revision: number } | null;
  queueing: boolean;
  activeWorkRunning: boolean;
  activeWorkNeedsInput: boolean;
  activeWorkRedirectable: boolean;
  stopping?: boolean;
  queuePaused?: boolean;
  onStop: () => void;
  onResumeQueue: () => void;
  queuedMessages: readonly QueuedMessage[];
  attachmentsAvailable: boolean;
  attachments: readonly ArtifactReference[];
  attachmentBusy: boolean;
  error: CommandError | null;
  onPromptChange: (prompt: string, intent?: ComposerEditIntent) => void;
  onPromptPaste: (text: string, start: number, end: number) => number;
  condensedPasteCount: number;
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
  dictation: requestedDictation,
  onOpenDictationSettings,
  contextActions,
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
  goalAvailable = false,
  goalMaxIterations = DEFAULT_GOAL_ITERATIONS,
  onGoalMaxIterationsChange = () => {},
  approvalMode,
  approvalModeVisible,
  approvalModeAvailable,
  approvalModeChanging,
  modelContext,
  onOpenModelSettings,
  canCompose,
  submitting,
  continuation,
  planRevision,
  queueing,
  activeWorkRunning,
  activeWorkNeedsInput,
  activeWorkRedirectable,
  stopping = false,
  queuePaused = false,
  onStop,
  onResumeQueue,
  queuedMessages,
  attachmentsAvailable,
  attachments,
  attachmentBusy,
  error,
  onPromptChange,
  onPromptPaste,
  condensedPasteCount,
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
  const dictation = requestedDictation;
  const draftReadOnly = Boolean(
    dictation?.state.phase === "recording" ||
    dictation?.state.busy ||
    dictation?.state.sending,
  );
  const [selectedSlashCommand, setSelectedSlashCommand] = useState<
    string | null
  >(null);
  const [dismissedSlashDraft, setDismissedSlashDraft] = useState<string | null>(
    null,
  );
  const roleMissing = role.trim().length === 0;
  const goalBlocked =
    mode === "goal" &&
    (!goalAvailable ||
      !validGoalIterations(goalMaxIterations) ||
      attachments.length > 0);
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
    !draftReadOnly &&
    slashCommandSuggestions.length > 0 &&
    dismissedSlashDraft !== prompt;
  const selectedSlashIndex = slashCommandSuggestions.findIndex(
    ({ command }) => command === selectedSlashCommand,
  );
  const activeSlashIndex =
    slashMenuOpen && slashCommandSuggestions.length > 0
      ? Math.max(0, selectedSlashIndex)
      : -1;
  const slashOptionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const editIntent = useRef<ComposerEditIntent | null>(null);
  const researchSourceSummary = researchSources
    .map(
      (source) =>
        RESEARCH_SOURCE_OPTIONS.find((option) => option.value === source)
          ?.label ?? source,
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
    if (draftReadOnly) {
      if (
        isComposerSendKey(
          { ...event, isComposing: event.nativeEvent.isComposing },
          "enter",
        )
      ) {
        event.preventDefault();
        formRef.current?.requestSubmit();
      }
      return;
    }
    if (event.key === "Backspace" || event.key === "Delete") {
      editIntent.current = {
        start: event.currentTarget.selectionStart,
        end: event.currentTarget.selectionEnd,
        inputType:
          event.key === "Backspace"
            ? "deleteContentBackward"
            : "deleteContentForward",
      };
    }
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
      isComposerSendKey(
        { ...event, isComposing: event.nativeEvent.isComposing },
        "enter",
      )
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
    <ConversationComposerFrame
      ref={formRef}
      className={`work-composer${mode === "plan" ? " is-plan-mode" : ""}${mode === "research" ? " is-research-mode" : ""}`}
      id="work-composer"
      aria-label="Send a prompt"
      onSubmit={onSubmit}
    >
      <div className="composer-header">
        <div className="composer-context">
          <Suspense
            fallback={
              <span className="composer-model-loading">Loading model…</span>
            }
          >
            <ComposerModelChip
              context={modelContext}
              role={role}
              onOpenSettings={onOpenModelSettings}
            />
          </Suspense>
          {contextActions}
        </div>
        <div className="composer-run-actions">
          {approvalModeVisible ? (
            <label
              className={`approval-mode-control mode-${approvalMode}`}
              title={
                approvalMode === "deny"
                  ? "Actions that need approval are blocked."
                  : approvalMode === "ask"
                    ? "Ask before actions that need approval."
                    : approvalMode === "risk_auto"
                      ? "Automatically approve eligible low-risk actions; ask about the rest."
                      : "Proceed without approval prompts. Policy and sandbox restrictions still apply."
              }
            >
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
                : mode === "goal"
                  ? `Goal: ${Number.isFinite(goalMaxIterations) ? goalMaxIterations : "?"} iterations`
                  : roleMissing
                    ? "Role required"
                    : "Run controls"}
            </summary>
            <div
              className={`run-controls-popover${mode === "research" ? " is-research" : ""}`}
            >
              {mode === "research" ? (
                <>
                  <ResearchControls
                    researchDepth={researchDepth}
                    researchSources={researchSources}
                    submitting={submitting}
                    onResearchDepthChange={onResearchDepthChange}
                    onResearchSourcesChange={onResearchSourcesChange}
                  />
                </>
              ) : (
                <>
                  {mode === "goal" ? (
                    <GoalControls
                      value={goalMaxIterations}
                      disabled={submitting}
                      onChange={onGoalMaxIterationsChange}
                    />
                  ) : null}
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
      </div>
      <div className="composer-body">
        {dictation?.state.sessionId || dictation?.state.phase === "starting" ? (
          <DictationRecordingBar {...dictation}>
            <DictationControl
              {...dictation}
              onSettings={onOpenDictationSettings}
              disabled={!canCompose || submitting}
            />
          </DictationRecordingBar>
        ) : null}
        {dictation?.state.error ? (
          <p className="inline-error" role="alert">
            {dictation.state.error}
          </p>
        ) : null}
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
                    pluginSkills?.find((skill) => skill.id === id)
                      ?.icon_data_url
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
            Unavailable conversation skills: {staleSelections.join(", ")}.
            Remove them or re-enable their plugin before sending.
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
        <NextUpQueue
          messages={queuedMessages}
          paused={queuePaused}
          resumeDisabled={!canCompose || activeWorkRunning || stopping}
          onResume={onResumeQueue}
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
        <ComposerInput
          ref={textareaRef}
          value={prompt}
          rows={2}
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
          aria-controls={
            slashMenuOpen ? "desktop-slash-command-menu" : undefined
          }
          aria-activedescendant={
            slashMenuOpen && activeSlashIndex >= 0
              ? `desktop-slash-command-${activeSlashIndex}`
              : undefined
          }
          aria-describedby={
            [
              promptOverLimit ? "prompt-byte-limit-error" : null,
              condensedPasteCount > 0 ? "composer-paste-summary" : null,
            ]
              .filter(Boolean)
              .join(" ") || undefined
          }
          disabled={!canCompose || submitting}
          readOnly={draftReadOnly}
          onKeyDown={handleKeyDown}
          onBeforeInput={(event) => {
            const textarea = event.currentTarget;
            const inputType = (event.nativeEvent as InputEvent).inputType;
            if (!inputType) return;
            editIntent.current = {
              start: textarea.selectionStart,
              end: textarea.selectionEnd,
              inputType,
            };
          }}
          onCut={(event) => {
            editIntent.current = {
              start: event.currentTarget.selectionStart,
              end: event.currentTarget.selectionEnd,
              inputType: "deleteByCut",
            };
          }}
          onPaste={(event) => {
            if (event.currentTarget.readOnly || event.currentTarget.disabled) {
              event.preventDefault();
              return;
            }
            const text = event.clipboardData.getData("text/plain");
            if (text.length === 0) return;
            event.preventDefault();
            editIntent.current = null;
            const textarea = event.currentTarget;
            const cursor = onPromptPaste(
              text,
              textarea.selectionStart,
              textarea.selectionEnd,
            );
            requestAnimationFrame(() => {
              textarea.focus();
              textarea.setSelectionRange(cursor, cursor);
            });
          }}
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
            onPromptChange(event.target.value, editIntent.current ?? undefined);
            editIntent.current = null;
          }}
        />
        {condensedPasteCount > 0 ? (
          <div
            className="composer-paste-summary"
            id="composer-paste-summary"
            role="status"
          >
            <IconFileText size={15} stroke={1.8} aria-hidden="true" />
            <span>
              {condensedPasteCount} large{" "}
              {condensedPasteCount === 1 ? "paste" : "pastes"} condensed. Full
              text is included when sent.
            </span>
          </div>
        ) : null}
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
      </div>
      <div className="composer-footer">
        <div className="composer-meta">
          <span>
            {slashCommandDraft
              ? "Slash commands run locally in Desktop and are never sent to the model."
              : mode === "research" && researchSources.length === 0
                ? "Select at least one evidence source before starting Research."
                : goalBlocked
                  ? "Goal requires an available target, 1–50 iterations, and a text-only prompt."
                  : activeWorkRunning
                    ? activeWorkNeedsInput
                      ? "Queued messages wait until the required response is resolved. Redirect stops this response and sends your guidance next."
                      : "Enter adds to Next up. Redirect stops this response and sends your guidance next."
                    : queueing
                      ? queuePaused
                        ? "Next up is paused. Resume when you are ready; your draft and queued messages are kept."
                        : "New messages join Next up. Resolve or remove a failed item to continue in order."
                      : mode === "plan"
                        ? planRevision === null
                          ? "Create a plan before making changes."
                          : "Revise this plan without making changes."
                        : mode === "research"
                          ? "Research your sources and return a report with citations."
                          : mode === "goal"
                            ? "Continue toward this objective within the Goal iteration limit."
                            : "Enter to send · Shift+Enter for a new line"}
          </span>
          {promptBytes >= promptByteLimit * 0.8 || promptOverLimit ? (
            <span
              className={promptOverLimit ? "counter-over-limit" : undefined}
            >
              {promptBytes.toLocaleString()} /{" "}
              {promptByteLimit.toLocaleString()} bytes
            </span>
          ) : null}
        </div>
        <div className="composer-action-row">
          {dictation &&
          !dictation.state.sessionId &&
          dictation.state.phase !== "starting" ? (
            <DictationControl
              {...dictation}
              onSettings={onOpenDictationSettings}
              disabled={!canCompose || submitting}
            />
          ) : null}
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
            </div>
          ) : null}
          <ComposerModeSwitch<RunMode>
            value={mode}
            onChange={onModeChange}
            disabled={submitting || planRevision !== null}
            options={[
              { value: "plan", label: "Plan" },
              { value: "execute", label: "Execute" },
              {
                value: "goal",
                label: "Goal",
                disabled: !goalAvailable,
                title: goalAvailable
                  ? "Continue toward an objective within an explicit iteration limit"
                  : "Goal mode requires an updated target with Goal tool access",
              },
              {
                value: "research",
                label: "Research",
                disabled: !researchAvailable,
                ...(researchAvailable
                  ? {}
                  : { title: "Research is unavailable for this target" }),
              },
            ]}
          />
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
                roleMissing ||
                goalBlocked
              }
              onClick={onRedirect}
            >
              <IconRouteAltLeft size={16} stroke={1.9} aria-hidden="true" />
              Redirect
            </button>
          ) : null}
          {!activeWorkRunning || prompt.trim().length > 0 ? (
            <ComposerSendButton
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
                    goalBlocked ||
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
              ) : undefined}
            </ComposerSendButton>
          ) : null}
          {activeWorkRunning ? (
            <button
              className={`send-button is-stop${stopping ? " is-stopping" : ""}`}
              type="button"
              aria-label={stopping ? "Stopping response" : "Stop response"}
              title={
                stopping
                  ? "Waiting for the response to stop"
                  : "Stop response and pause queued messages"
              }
              disabled={!canCompose || !activeWorkRedirectable || stopping}
              onClick={onStop}
            >
              {stopping ? (
                <span className="spinner" aria-hidden="true" />
              ) : (
                <IconPlayerStopFilled size={16} aria-hidden="true" />
              )}
            </button>
          ) : null}
        </div>
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
    </ConversationComposerFrame>
  );
}
