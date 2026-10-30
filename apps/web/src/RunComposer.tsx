import {
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  useId,
} from "react";
import { Button, ComposerModeSwitch, DropdownSelect } from "@colossus/ui";
import { Badge } from "@colossus/ui/components/ui/badge";
import { ConversationComposer, NextPromptSuggestion, useNextPrompt, type NextPromptSource, type NextPromptPrediction } from "@colossus/ui/conversation";
import { type FleetNode } from "./api";
import { HostedAttachments, type HostedAttachment } from "./HostedAttachments";
import { workspaceName } from "./workspace-navigation";
import { NextPromptModeContext, SendShortcutContext } from "./Appearance";
import "@colossus/ui/styles/next-prompt.css";
import {
  ResearchControls,
  RESEARCH_SOURCE_OPTIONS,
} from "@colossus/ui/components/ResearchControls";
import type {
  ResearchDepth,
  ResearchSourceKind,
} from "@colossus/ui/session/types";
import { useRunModeCapabilities } from "./research-capability";
import { ComposerModelPicker as ModelPicker } from "./ComposerModelPicker";
import {
  REASONING_EFFORTS,
  reasoningEffort,
  type ReasoningEffort,
} from "./model-catalog";
import {
  GoalControls,
  DEFAULT_GOAL_ITERATIONS,
  validGoalIterations,
} from "@colossus/ui/components/GoalControls";
import "@colossus/ui/styles/research-controls.css";
import {
  IconAdjustmentsHorizontal,
  IconShieldCheck,
} from "@tabler/icons-react";
import {
  planAction,
  planPrompt,
  type PlanContinuation,
  type PlanRunAction,
} from "./plan-continuation";

export interface RunRequest {
  model_profile?: string;
  reasoning_effort?: ReasoningEffort;
  plugin_skill_ids: string[];
  input: (
    { text: string; artifact?: never } | { artifact: string; text?: never }
  )[];
  session_id: null;
  end_user_id: null;
  role: string;
  mode: string;
  goal_max_iterations?: number;
  research_depth: ResearchDepth | null;
  research_sources: ResearchSourceKind[];
  plan_action: PlanRunAction | null;
  branch: null;
  max_turns: number;
  idempotency_key: string;
}
interface SubmissionAttempt {
  signature: string;
  intent: string;
  request: RunRequest;
}

export function RunComposer({
  nodes,
  nodeId,
  lockedRuntime = false,
  disabled = false,
  busy,
  label,
  action,
  onNodeChange,
  onSubmit,
  onPolicy,
  draftSeed,
  planContinuation,
  onCancelPlan,
  runtimeLabel,
  initialRequest,
  running = false,
  queuedTurns = 0,
  onStop,
  stopping = false,
  stopDisabled = false,
  hostedProject,
  conversationBranch = false,
  nextPromptSource,
  loadPromptPrediction,
}: {
  nodes: FleetNode[];
  nodeId: string;
  lockedRuntime?: boolean;
  disabled?: boolean;
  busy: boolean;
  label: string;
  action: string;
  onNodeChange?: (id: string) => void;
  onSubmit: (request: RunRequest) => Promise<boolean>;
  onPolicy?: (() => void) | undefined;
  draftSeed?: { key: string; text: string } | undefined;
  planContinuation?: PlanContinuation | null;
  onCancelPlan?: () => void;
  runtimeLabel?: string | undefined;
  initialRequest?: RunRequest | undefined;
  running?: boolean;
  queuedTurns?: number;
  onStop?: (() => void) | undefined;
  stopping?: boolean;
  stopDisabled?: boolean;
  hostedProject?: string | undefined;
  conversationBranch?: boolean;
  nextPromptSource?: NextPromptSource | null | undefined;
  loadPromptPrediction?: ((signal: AbortSignal) => Promise<NextPromptPrediction | null>) | undefined;
}) {
  const modeGroup = useId();
  const [attachments, setAttachments] = useState<HostedAttachment[]>([]);
  const [uploading, setUploading] = useState(false);
  const [draft, setDraft] = useState(""),
    [mode, setMode] = useState(initialRequest?.mode ?? "execute"),
    [role, setRole] = useState(initialRequest?.role ?? "primary"),
    [goalMaxIterations, setGoalMaxIterations] = useState(
      initialRequest?.goal_max_iterations ?? DEFAULT_GOAL_ITERATIONS,
    ),
    [researchDepth, setResearchDepth] = useState<ResearchDepth>(
      initialRequest?.research_depth ?? "standard",
    ),
    [researchSources, setResearchSources] = useState<ResearchSourceKind[]>([
      ...(initialRequest?.research_sources.length
        ? initialRequest.research_sources
        : ["repo" as const]),
    ]);
  const draftRevision = useRef(0);
  const updateDraft = useCallback((value: string) => {
    draftRevision.current++;
    setDraft(value);
  }, []);
  const input = useRef<HTMLTextAreaElement>(null);
  const previousPlan = useRef(false);
  useEffect(() => {
    if (planContinuation) {
      setMode(planContinuation.strategy ? "execute" : "plan");
      setRole(planContinuation.role);
      updateDraft(planPrompt(planContinuation));
      input.current?.focus();
    } else if (previousPlan.current) {
      setMode("execute");
      updateDraft("");
    }
    previousPlan.current = Boolean(planContinuation);
  }, [planContinuation, updateDraft]);
  useEffect(() => {
    if (draftSeed) {
      updateDraft(draftSeed.text);
      // A draft can open from a menu whose focus scope is still closing.
      const frame = requestAnimationFrame(() => input.current?.focus());
      return () => cancelAnimationFrame(frame);
    }
  }, [draftSeed?.key, draftSeed?.text, updateDraft]);
  const attempt = useRef<SubmissionAttempt | null>(null);
  const [failedAttempt, setFailedAttempt] = useState<SubmissionAttempt | null>(
    null,
  );
  const [selectionRevision, setSelectionRevision] = useState(0);
  const target = nodes.find((item) => item.node.node_id === nodeId);
  const shortcut = useContext(SendShortcutContext);
  const predictionMode = useContext(NextPromptModeContext);
  const nextPrompt = useNextPrompt({ source: nextPromptSource, mode: predictionMode, draft, busy: busy || running || disabled || uploading || Boolean(planContinuation) || attachments.length > 0, load: target?.presence?.ready && target.presence.capabilities.includes("composer.prediction.v1") ? loadPromptPrediction : undefined });
  const acceptNextPrompt = () => {
    if (!draft && nextPrompt.text && !busy && !running && !disabled) { updateDraft(nextPrompt.text); input.current?.focus(); }
  };
  const posture = target?.node.policy;
  const capabilities = useRunModeCapabilities(target, disabled);
  const [modelChoice, setModelChoice] = useState<{
    scope: string;
    profile: string;
    effort: ReasoningEffort | "";
  }>({
    scope: capabilities.modelScope,
    profile: initialRequest?.model_profile ?? "",
    effort: initialRequest?.reasoning_effort ?? "",
  });
  const modelScope = capabilities.modelScope;
  const profile = modelChoice.scope === modelScope ? modelChoice.profile : "";
  const effort = modelChoice.scope === modelScope ? modelChoice.effort : "";
  useEffect(() => {
    setModelChoice((current) =>
      current.scope === modelScope
        ? current
        : { scope: modelScope, profile: "", effort: "" },
    );
  }, [modelScope]);
  const selectedModel = capabilities.modelProfiles.find(
    (model) => model.profile === profile,
  );
  const defaultModel = capabilities.modelProfiles.find((model) =>
    model.default_roles.includes(role),
  );
  const modelBlocked =
    mode !== "research" &&
    (Boolean(profile && !selectedModel) ||
      Boolean(effort && !capabilities.modelProfiles.length));
  const researchAvailable = capabilities.research;
  const goalBlocked =
    mode === "goal" &&
    (!capabilities.goal || !validGoalIterations(goalMaxIterations));
  const tools = posture?.allowed_tools ?? [];
  const availableSources = RESEARCH_SOURCE_OPTIONS.filter(
    (option) =>
      tools.includes("*") ||
      tools.includes(
        { repo: "filesystem.search", web: "web.search", mcp: "mcp.call" }[
          option.value
        ],
      ),
  ).map((option) => option.value);
  const sources = RESEARCH_SOURCE_OPTIONS.map((option) => option.value).filter(
    (source) =>
      researchSources.includes(source) && availableSources.includes(source),
  );
  const researchBlocked =
    mode === "research" && (!researchAvailable || !sources.length);
  const planBlocked = Boolean(
    planContinuation &&
    (!capabilities.plans ||
      (planContinuation.strategy &&
        planContinuation.strategy !== "direct" &&
        (!capabilities.goal ||
          !validGoalIterations(
            planContinuation.strategy.goal.max_iterations,
          )))),
  );
  // Catalog epochs can clear displayed choices after reconnecting, but cannot
  // change a failed durable submission. Only another human edit starts a new one.
  const intent = JSON.stringify({
    draft,
    attachments: attachments.map((file) => file.artifact_id),
    project: target?.node.project_id,
    nodeId,
    mode,
    selectionRevision,
    planRole: planContinuation?.role,
    planAction: planContinuation ? planAction(planContinuation) : null,
    goalMaxIterations: mode === "goal" ? goalMaxIterations : null,
    researchDepth: mode === "research" ? researchDepth : null,
    researchSources: mode === "research" ? researchSources : [],
  });
  const retained = failedAttempt?.intent === intent ? failedAttempt : null;
  const modelLabel =
    posture?.models.length === 1
      ? posture.models[0]?.label
      : posture?.models.length
        ? "Agent-routed model"
        : "Model not reported";
  const policyStale =
    !target?.presence?.ready ||
    !target.node.policy_observed_at ||
    Date.now() / 1000 - target.node.policy_observed_at > 60;
  const roles = target?.node.roles ?? [];
  useEffect(() => {
    if (roles.length && !roles.includes(role)) setRole(roles[0]!);
  }, [roles, role]);
  async function send() {
    if (
      !draft.trim() ||
      !nodeId ||
      disabled ||
      busy ||
      uploading ||
      planBlocked ||
      (!retained && (researchBlocked || goalBlocked || modelBlocked))
    )
      return;
    const signature = JSON.stringify({
      intent,
      modelProfile: mode === "research" ? "" : profile,
      reasoningEffort: mode === "research" ? "" : effort,
    });
    if (retained) attempt.current = retained;
    else if (attempt.current?.signature !== signature) {
      attempt.current = {
        signature,
        intent,
        request: {
          ...(mode !== "research" && profile ? { model_profile: profile } : {}),
          ...(mode !== "research" && effort
            ? { reasoning_effort: effort }
            : {}),
          plugin_skill_ids: [],
          input: [
            { text: draft },
            ...attachments.map((file) => ({ artifact: file.artifact_id })),
          ],
          session_id: null,
          end_user_id: null,
          role: planContinuation?.role ?? role,
          mode,
          ...(mode === "goal"
            ? { goal_max_iterations: goalMaxIterations }
            : {}),
          research_depth: mode === "research" ? researchDepth : null,
          research_sources: mode === "research" ? sources : [],
          plan_action: planContinuation ? planAction(planContinuation) : null,
          branch: null,
          max_turns: 24,
          idempotency_key: crypto.randomUUID(),
        },
      };
    }
    const submitted = attempt.current;
    const submittedRevision = draftRevision.current;
    const submittedAttachments = new Set(
      attachments.map((file) => file.artifact_id),
    );
    if (await onSubmit(submitted.request)) {
      if (draftRevision.current === submittedRevision) updateDraft("");
      setAttachments((current) =>
        current.filter((file) => !submittedAttachments.has(file.artifact_id)),
      );
      if (attempt.current === submitted) attempt.current = null;
      setFailedAttempt(null);
      input.current?.focus();
    } else setFailedAttempt(submitted);
  }
  return (
    <ConversationComposer
      value={draft}
      onChange={updateDraft}
      textareaRef={input}
      className={
        planContinuation ? "run-composer is-plan-continuation" : "run-composer"
      }
      label={label}
      action={
        busy
          ? "Submitting…"
          : planContinuation
            ? planContinuation.strategy
              ? "Approve and execute plan"
              : "Revise plan"
            : action
      }
      placeholder={
        disabled
          ? "This conversation is available to read."
          : "What would you like to work on?"
      }
      disabled={
        disabled ||
        !nodeId ||
        planBlocked ||
        (!retained && (researchBlocked || goalBlocked || modelBlocked))
      }
      busy={busy || uploading}
      running={running}
      onStop={onStop}
      stopping={stopping}
      stopDisabled={stopDisabled}
      shortcut={shortcut}
      textareaProps={{ onKeyDown: event => {
        if (event.key === "Tab" && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey && nextPrompt.text) { event.preventDefault(); acceptNextPrompt(); }
        else if (event.key === "Escape" && nextPrompt.text) { event.preventDefault(); nextPrompt.dismiss(); }
      } }}
      context={
        <>
          {planContinuation ? (
            <span className="composer-plan-selection">
              Plan revision {planContinuation.plan.revision} ·{" "}
              {planContinuation.strategy
                ? "Approve and execute"
                : "Revising draft"}
              <Badge className="composer-fixed-role">
                Role: {planContinuation.role}
              </Badge>
              <Button variant="tertiary" disabled={busy} onClick={onCancelPlan}>
                Cancel plan action
              </Button>
            </span>
          ) : null}
          <span className="composer-context-runtime">
            {runtimeLabel ??
              (target ? workspaceName(target) : "Select a workspace")}
          </span>
          {mode === "research" ? (
            <span
              className="composer-model"
              title="Research uses the model configured for the host’s research route."
            >
              Host research model
            </span>
          ) : capabilities.modelProfiles.length ? (
            <ModelPicker
              disabled={disabled || busy}
              model={profile}
              label={
                selectedModel?.model ?? defaultModel?.model ?? "Role default"
              }
              detail={
                selectedModel?.provider ??
                defaultModel?.provider ??
                "Runtime role default"
              }
              models={[
                {
                  value: "",
                  label: "Role default",
                  detail: defaultModel
                    ? `${defaultModel.model} · ${defaultModel.provider}`
                    : "Use the runtime’s configured role route",
                },
                ...capabilities.modelProfiles.map((model) => ({
                  value: model.profile,
                  label: model.model,
                  detail: `${model.provider} · ${model.profile} (${model.provider_profile})`,
                })),
              ]}
              onModelChange={(next) => {
                const differsFromRetained =
                  retained &&
                  ((retained.request.model_profile ?? "") !== next ||
                    (retained.request.reasoning_effort ?? "") !== effort);
                if (next !== profile || differsFromRetained) {
                  setSelectionRevision((value) => value + 1);
                  setModelChoice({
                    scope: modelScope,
                    profile: next,
                    effort: "",
                  });
                }
              }}
              reasoning={effort}
              reasoningLabel={
                effort ||
                `Model default${(selectedModel ?? defaultModel)?.reasoning_effort ? ` · ${(selectedModel ?? defaultModel)!.reasoning_effort}` : ""}`
              }
              reasoningOptions={[
                { value: "", label: "Model default" },
                ...REASONING_EFFORTS.map((value) => ({ value, label: value })),
              ]}
              onReasoningChange={(value) => {
                const next = reasoningEffort(value) ?? "";
                if (next === effort) return;
                setSelectionRevision((revision) => revision + 1);
                setModelChoice({
                  scope: modelScope,
                  profile,
                  effort: next,
                });
              }}
            />
          ) : (
            <span
              className="composer-model"
              title={`Model routing is configured on the host${posture?.models.length ? `: ${posture.models.map((model) => `${model.profile}: ${model.label}`).join(" · ")}` : ""}`}
            >
              {capabilities.modelLoading
                ? "Loading configured models…"
                : modelLabel}
            </span>
          )}
          {onPolicy ? (
            <Button
              variant="tertiary"
              className="composer-policy-pill"
              onClick={onPolicy}
            >
              <IconShieldCheck size={14} aria-hidden="true" />
              {posture
                ? `Approvals: ${posture.approval_mode.replaceAll("_", " ")}`
                : "Policy not reported"}
              {posture && policyStale ? " · Stale" : ""}
            </Button>
          ) : (
            <span className="composer-policy-pill">
              {posture
                ? `Approvals: ${posture.approval_mode.replaceAll("_", " ")}`
                : "Policy not reported"}
            </span>
          )}
        </>
      }
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
      controls={
        <>
          {hostedProject ? (
            <HostedAttachments
              key={hostedProject}
              project={hostedProject}
              files={attachments}
              onChange={setAttachments}
              disabled={disabled || busy || Boolean(planContinuation)}
              onBusy={setUploading}
            />
          ) : null}
          {!lockedRuntime ? (
            <DropdownSelect
              aria-label="Execution agent"
              value={nodeId}
              onChange={(event) => onNodeChange?.(event.target.value)}
              disabled={disabled || busy}
            >
              {nodes
                .filter((item) => !item.node.revoked)
                .map((item) => (
                  <option key={item.node.node_id} value={item.node.node_id}>
                    {workspaceName(item)}
                    {item.node.workspace_label
                      ? ` · ${item.node.workspace_label}`
                      : ""}
                    {item.presence?.ready ? "" : " · Offline"}
                  </option>
                ))}
            </DropdownSelect>
          ) : (
            <span className="composer-target">
              {target ? workspaceName(target) : "Assigned workspace"}
            </span>
          )}
          <ComposerModeSwitch
            name={modeGroup}
            value={mode}
            onChange={setMode}
            disabled={disabled || busy || Boolean(planContinuation)}
            options={[
              { value: "plan", label: "Plan" },
              { value: "execute", label: "Execute" },
              {
                value: "goal",
                label: "Goal",
                disabled: conversationBranch || !capabilities.goal,
                title: capabilities.goal
                  ? "Continue toward an objective within an explicit iteration limit"
                  : "Connect a runtime with Goal support and authorized Goal tools.",
              },
              {
                value: "research",
                label: "Research",
                disabled: conversationBranch || !researchAvailable,
                title: researchAvailable
                  ? "Run a bounded evidence-and-citation task"
                  : "Connect a runtime that advertises Research support.",
              },
            ]}
          />
          {mode === "goal" ? (
            <details className="research-run-controls">
              <summary>
                <IconAdjustmentsHorizontal size={16} aria-hidden="true" />
                Goal:{" "}
                {Number.isFinite(goalMaxIterations)
                  ? goalMaxIterations
                  : "?"}{" "}
                iterations
              </summary>
              <div className="run-controls-popover">
                <GoalControls
                  value={goalMaxIterations}
                  disabled={disabled || busy}
                  onChange={setGoalMaxIterations}
                />
              </div>
            </details>
          ) : null}
          {mode === "research" ? (
            <details className="research-run-controls">
              <summary>
                <IconAdjustmentsHorizontal size={16} aria-hidden="true" />
                Sources:{" "}
                {sources
                  .map(
                    (source) =>
                      RESEARCH_SOURCE_OPTIONS.find(
                        (option) => option.value === source,
                      )!.label,
                  )
                  .join(", ") || "None"}
              </summary>
              <div className="run-controls-popover is-research">
                <ResearchControls
                  researchDepth={researchDepth}
                  researchSources={sources}
                  availableSources={availableSources}
                  submitting={busy || disabled}
                  onResearchDepthChange={setResearchDepth}
                  onResearchSourcesChange={setResearchSources}
                />
              </div>
            </details>
          ) : null}
          {!planContinuation ? (
            <DropdownSelect
              aria-label="Agent role"
              value={role}
              onChange={(event) => {
                if (event.target.value !== role) {
                  setSelectionRevision((revision) => revision + 1);
                  setRole(event.target.value);
                }
              }}
              disabled={disabled || busy}
            >
              {roles.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </DropdownSelect>
          ) : null}
          <span className="composer-hint">
            {shortcut === "enter" ? "Enter ↵ · Shift ↵ newline" : "⌘ / Ctrl ↵"}
          </span>
        </>
      }
      queue={
        queuedTurns > 0 ? (
          <p className="shared-composer-notice" role="status">
            {queuedTurns} queued {queuedTurns === 1 ? "message" : "messages"}
          </p>
        ) : undefined
      }
      notice={
        retained
          ? "The previous submission was not confirmed. Sending again reconciles its original settings."
          : planBlocked
            ? "Plan continuation requires the original personal runtime with Plan support. Reconnect or cancel this action."
            : modelBlocked
              ? "The selected model is unavailable on this runtime. Choose a configured model or use the role default."
              : goalBlocked
                ? "Goal requires an available runtime and an iteration limit from 1 to 50."
                : researchBlocked
                  ? "Research requires an available runtime and at least one authorized evidence source."
                  : target && !target.presence?.ready && !target.node.revoked
                    ? "This agent is offline. Accepted messages remain queued until it reconnects."
                    : nextPrompt.text || nextPrompt.auxiliaryTokens != null ? <NextPromptSuggestion text={nextPrompt.text} onAccept={acceptNextPrompt} onDismiss={nextPrompt.dismiss} auxiliaryTokens={nextPrompt.auxiliaryTokens} /> : undefined
      }
    />
  );
}
