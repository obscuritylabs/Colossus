import { useContext, useEffect, useRef, useState } from "react";
import { Button, ComposerModeSwitch, DropdownSelect } from "@colossus/ui";
import { ConversationComposer } from "@colossus/ui/conversation";
import { type FleetNode } from "./api";
import { workspaceName } from "./workspace-navigation";
import { SendShortcutContext } from "./Appearance";
import {
  ResearchControls,
  RESEARCH_SOURCE_OPTIONS,
} from "@colossus/ui/components/ResearchControls";
import type {
  ResearchDepth,
  ResearchSourceKind,
} from "@colossus/ui/session/types";
import { useRunModeCapabilities } from "./research-capability";
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

export interface RunRequest {
  plugin_skill_ids: string[];
  input: { text: string }[];
  session_id: null;
  end_user_id: null;
  role: string;
  mode: string;
  goal_max_iterations?: number;
  research_depth: ResearchDepth | null;
  research_sources: ResearchSourceKind[];
  plan_action: null;
  branch: null;
  max_turns: number;
  idempotency_key: string;
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
}) {
  const [draft, setDraft] = useState(""),
    [mode, setMode] = useState("execute"),
    [role, setRole] = useState("primary"),
    [goalMaxIterations, setGoalMaxIterations] = useState(
      DEFAULT_GOAL_ITERATIONS,
    ),
    [researchDepth, setResearchDepth] = useState<ResearchDepth>("standard"),
    [researchSources, setResearchSources] = useState<ResearchSourceKind[]>([
      "repo",
    ]);
  const input = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (draftSeed) {
      setDraft(draftSeed.text);
      input.current?.focus();
    }
  }, [draftSeed?.key, draftSeed?.text]);
  const attempt = useRef<{ signature: string; key: string } | null>(null);
  const target = nodes.find((item) => item.node.node_id === nodeId);
  const shortcut = useContext(SendShortcutContext);
  const posture = target?.node.policy;
  const capabilities = useRunModeCapabilities(target, disabled);
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
      researchBlocked ||
      goalBlocked
    )
      return;
    const signature = JSON.stringify({
      draft,
      nodeId,
      mode,
      role,
      goalMaxIterations: mode === "goal" ? goalMaxIterations : null,
      researchDepth: mode === "research" ? researchDepth : null,
      researchSources: mode === "research" ? sources : [],
    });
    if (attempt.current?.signature !== signature)
      attempt.current = { signature, key: crypto.randomUUID() };
    if (
      await onSubmit({
        plugin_skill_ids: [],
        input: [{ text: draft }],
        session_id: null,
        end_user_id: null,
        role,
        mode,
        ...(mode === "goal" ? { goal_max_iterations: goalMaxIterations } : {}),
        research_depth: mode === "research" ? researchDepth : null,
        research_sources: mode === "research" ? sources : [],
        plan_action: null,
        branch: null,
        max_turns: 24,
        idempotency_key: attempt.current.key,
      })
    ) {
      setDraft("");
      attempt.current = null;
      input.current?.focus();
    }
  }
  return (
    <ConversationComposer
      value={draft}
      onChange={setDraft}
      textareaRef={input}
      className="run-composer"
      label={label}
      action={busy ? "Submitting…" : action}
      placeholder={
        disabled
          ? "This conversation is available to read."
          : "What would you like to work on?"
      }
      disabled={disabled || !nodeId || researchBlocked || goalBlocked}
      busy={busy}
      shortcut={shortcut}
      context={
        <>
          <span className="composer-context-runtime">
            {target ? workspaceName(target) : "Select a workspace"}
          </span>
          <span
            className="composer-model"
            title={`Model routing is configured on the host${posture?.models.length ? `: ${posture.models.map((model) => `${model.profile}: ${model.label}`).join(" · ")}` : ""}`}
          >
            {modelLabel}
          </span>
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
            value={mode}
            onChange={setMode}
            disabled={disabled || busy}
            options={[
              { value: "plan", label: "Plan" },
              { value: "execute", label: "Execute" },
              {
                value: "goal",
                label: "Goal",
                disabled: !capabilities.goal,
                title: capabilities.goal
                  ? "Continue toward an objective within an explicit iteration limit"
                  : "Connect a runtime with Goal support and authorized Goal tools.",
              },
              {
                value: "research",
                label: "Research",
                disabled: !researchAvailable,
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
          <DropdownSelect
            aria-label="Agent role"
            value={role}
            onChange={(event) => setRole(event.target.value)}
            disabled={disabled || busy}
          >
            {roles.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </DropdownSelect>
          <span className="composer-hint">
            {shortcut === "enter" ? "Enter ↵ · Shift ↵ newline" : "⌘ / Ctrl ↵"}
          </span>
        </>
      }
      notice={
        goalBlocked
          ? "Goal requires an available runtime and an iteration limit from 1 to 50."
          : researchBlocked
            ? "Research requires an available runtime and at least one authorized evidence source."
            : target && !target.presence?.ready && !target.node.revoked
              ? "This agent is offline. Accepted messages remain queued until it reconnects."
              : undefined
      }
    />
  );
}
