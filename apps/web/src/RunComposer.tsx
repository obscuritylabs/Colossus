import { useContext, useEffect, useRef, useState } from "react";
import { Button, ComposerModeSwitch, DropdownSelect } from "@colossus/ui";
import { ConversationComposer } from "@colossus/ui/conversation";
import { type FleetNode } from "./api";
import { SendShortcutContext } from "./Appearance";
import { IconShieldCheck } from "@tabler/icons-react";

export interface RunRequest {
  plugin_skill_ids: string[];
  input: { text: string }[];
  session_id: null;
  end_user_id: null;
  role: string;
  mode: string;
  research_depth: null;
  research_sources: string[];
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
    [role, setRole] = useState("primary");
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
    if (!draft.trim() || !nodeId || disabled || busy) return;
    const signature = JSON.stringify({ draft, nodeId, mode, role });
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
        research_depth: null,
        research_sources: [],
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
      disabled={disabled || !nodeId}
      busy={busy}
      shortcut={shortcut}
      context={
        <>
          <span className="composer-context-runtime">
            {target?.node.label ?? "Select an agent"}
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
                    {item.node.label}
                    {item.node.workspace_label
                      ? ` · ${item.node.workspace_label}`
                      : ""}
                    {item.presence?.ready ? "" : " · Offline"}
                  </option>
                ))}
            </DropdownSelect>
          ) : (
            <span className="composer-target">
              {target?.node.label ?? "Assigned agent"}
            </span>
          )}
          <ComposerModeSwitch
            value={mode}
            onChange={setMode}
            disabled={disabled || busy}
            options={[
              { value: "execute", label: "Execute" },
              { value: "plan", label: "Plan" },
            ]}
          />
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
        target && !target.presence?.ready && !target.node.revoked
          ? "This agent is offline. Accepted messages remain queued until it reconnects."
          : undefined
      }
    />
  );
}
