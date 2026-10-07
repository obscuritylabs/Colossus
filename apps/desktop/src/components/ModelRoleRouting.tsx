import { useId } from "react";
import { DropdownSelect } from "./DropdownSelect";
import "./model-role-routing.css";

export const MODEL_ROLES = [
  {
    id: "primary",
    label: "Primary",
    description:
      "Runs everyday conversations and agent tasks, including planning and tool use. Also handles any role left on “Use primary.” Choose a capable general-purpose model.",
  },
  {
    id: "risk_evaluator",
    label: "Risk evaluator",
    description:
      "Assesses proposed actions for low-risk automatic approval under your configured policy. Choose a model with careful judgment and reliable instruction following.",
  },
  {
    id: "context_summarizer",
    label: "Context summarizer",
    description:
      "Summarizes older conversation history when model-assisted compaction is enabled, preserving key decisions and unfinished work. Choose a model with accurate summaries and enough context capacity.",
  },
  {
    id: "subagent_default",
    label: "Default subagent",
    description:
      "Runs tasks delegated to child agents. Choose a model that handles the tools and complexity of your delegated work, balancing quality, speed, and cost.",
  },
  {
    id: "research_planner",
    label: "Research planner",
    description:
      "Turns a research question into focused search queries. Choose a model that can break down broad questions and follow structured output instructions.",
  },
  {
    id: "research_worker",
    label: "Research worker",
    description:
      "Reads collected sources and extracts factual claims for the research question. Choose a model with strong reading accuracy; speed and cost matter when processing many sources.",
  },
  {
    id: "research_synthesizer",
    label: "Research synthesizer",
    description:
      "Combines source-backed findings into the final research report, with citations and limitations. Choose a model with strong synthesis, clear writing, and enough context for the gathered evidence.",
  },
] as const;

export interface RoleModel {
  profile: string;
  label: string;
  model: string;
  providerProfile: string;
}

/** One role editor shared by the routing matrix and graph inspector. */
export function ModelRoleSelect({
  roleId,
  roles,
  models,
  onChange,
  disabled = false,
  id,
  describedBy,
  label,
}: {
  roleId: string;
  roles: Record<string, string>;
  models: RoleModel[];
  onChange: (roles: Record<string, string>) => void;
  disabled?: boolean;
  id?: string;
  describedBy?: string;
  label: string;
}) {
  const primary = roleId === "primary";
  const assigned = roles[roleId] ?? "";
  const unavailable =
    assigned && !models.some((model) => model.profile === assigned);
  return (
    <DropdownSelect
      {...(id ? { id } : {})}
      aria-label={label}
      {...(describedBy ? { "aria-describedby": describedBy } : {})}
      required={primary}
      disabled={disabled || !models.length}
      value={assigned}
      onChange={(event) => {
        const next = { ...roles };
        if (event.target.value) next[roleId] = event.target.value;
        else delete next[roleId];
        onChange(next);
      }}
    >
      <option value="" disabled={primary}>
        {primary ? "Choose a model" : "Use primary"}
      </option>
      {unavailable ? (
        <option value={assigned} disabled>
          Unavailable model: {assigned}
        </option>
      ) : null}
      {models.map((model) => (
        <option key={model.profile} value={model.profile}>
          {model.label} · {model.model}
        </option>
      ))}
    </DropdownSelect>
  );
}

export function ModelRoleRouting({
  roles,
  models,
  onChange,
  disabled = false,
}: {
  roles: Record<string, string>;
  models: RoleModel[];
  onChange: (roles: Record<string, string>) => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <section className="model-role-routing" aria-labelledby={`${id}-heading`}>
      <header>
        <h3 id={`${id}-heading`}>Role routing</h3>
        <p>
          Choose a primary model. Specialized roles use it unless you assign a
          different model.
        </p>
      </header>
      {!models.length ? (
        <p className="model-role-empty">
          Select a model for this workspace to configure routing.
        </p>
      ) : null}
      <div className="model-role-list">
        {MODEL_ROLES.map((role) => {
          const primary = role.id === "primary";
          const assigned = roles[role.id] ?? "";
          const effectiveProfile = assigned || (primary ? "" : roles.primary);
          const effective = models.find(
            (model) => model.profile === effectiveProfile,
          );
          return (
            <div
              className={`model-role-row${primary ? " model-role-primary" : ""}`}
              key={role.id}
            >
              <div className="model-role-purpose">
                <div>
                  <label htmlFor={`${id}-${role.id}`}>{role.label}</label>
                  {primary ? (
                    <span className="model-role-required">Required</span>
                  ) : null}
                </div>
                <p id={`${id}-${role.id}-description`}>{role.description}</p>
                <code>{role.id}</code>
              </div>
              <div className="model-role-selection">
                <ModelRoleSelect
                  id={`${id}-${role.id}`}
                  roleId={role.id}
                  roles={roles}
                  models={models}
                  label={`${role.label} model`}
                  describedBy={`${id}-${role.id}-description ${id}-${role.id}-effective`}
                  disabled={disabled}
                  onChange={onChange}
                />
                <small id={`${id}-${role.id}-effective`}>
                  {effective
                    ? `Uses ${effective.model} · ${effective.providerProfile}`
                    : assigned
                      ? "Select an available model."
                      : "Choose a primary model."}
                </small>
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
