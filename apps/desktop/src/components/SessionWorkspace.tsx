import {
  IconAdjustments,
  IconArrowsMaximize,
  IconBook2,
  IconCheck,
  IconChecklist,
  IconChevronDown,
  IconChevronRight,
  IconExternalLink,
  IconFileText,
  IconFolder,
  IconFolders,
  IconListCheck,
  IconListDetails,
  IconMessageCircle,
  IconRobot,
  IconScale,
  IconSearch,
  IconSparkles,
  IconTargetArrow,
  IconTopologyStar3,
} from "@tabler/icons-react";
import { useMemo, useRef, useState } from "react";
import type { ComponentType, CSSProperties } from "react";

import { shortDateLabel } from "../presenters";
import { selectSessionPlans, selectSessionSources } from "../session-resources";
import type { SessionPlanReference } from "../session-resources";
import type { RunView } from "../state";
import type { SessionMap, SessionMapResource } from "../types";
import type { AgentParticipant, AgentWorkState } from "./AgentFlow";
import type { ArtifactViewItem } from "./ArtifactWorkspace";
import { isWebUri, workspaceSourcePath } from "./ResearchSourcesPanel";

export type SessionWorkspaceView =
  "conversation" | "topology" | "plans" | "sources" | "resources";

const SESSION_TABS: ReadonlyArray<{
  id: SessionWorkspaceView;
  label: string;
}> = [
  { id: "conversation", label: "Conversation" },
  { id: "topology", label: "Topology" },
  { id: "plans", label: "Plans" },
  { id: "sources", label: "Sources" },
  { id: "resources", label: "Resources" },
];

export function SessionWorkspaceTabs({
  active,
  onChange,
}: {
  active: SessionWorkspaceView;
  onChange: (view: SessionWorkspaceView) => void;
}) {
  return (
    <nav className="session-workspace-tabs" aria-label="Session views">
      {SESSION_TABS.map((tab) => (
        <button
          key={tab.id}
          type="button"
          aria-current={active === tab.id ? "page" : undefined}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
        </button>
      ))}
    </nav>
  );
}

function readableState(state: AgentWorkState): string {
  return state.charAt(0).toUpperCase() + state.slice(1);
}

type SessionMapFamily =
  | "delegates"
  | "goals"
  | "tasks"
  | "plans"
  | "decisions"
  | "memories"
  | "research"
  | "sources"
  | "artifacts";

type SessionMapLayer = "agents" | "work" | "context" | "research" | "outputs";

const FAMILY_META: ReadonlyArray<{
  id: SessionMapFamily;
  label: string;
  layer: SessionMapLayer;
  icon: ComponentType<{ size?: number; stroke?: number }>;
}> = [
  {
    id: "delegates",
    label: "Delegated agents",
    layer: "agents",
    icon: IconRobot,
  },
  { id: "goals", label: "Goals", layer: "work", icon: IconTargetArrow },
  { id: "tasks", label: "Tasks", layer: "work", icon: IconChecklist },
  { id: "plans", label: "Plans", layer: "work", icon: IconListDetails },
  {
    id: "decisions",
    label: "Key decisions",
    layer: "context",
    icon: IconScale,
  },
  { id: "memories", label: "Memories", layer: "context", icon: IconBook2 },
  { id: "research", label: "Research", layer: "research", icon: IconSearch },
  { id: "sources", label: "Sources", layer: "research", icon: IconFileText },
  { id: "artifacts", label: "Artifacts", layer: "outputs", icon: IconFolders },
];

const LAYER_META: ReadonlyArray<{ id: SessionMapLayer; label: string }> = [
  { id: "agents", label: "Agents" },
  { id: "work", label: "Work" },
  { id: "context", label: "Context" },
  { id: "research", label: "Research" },
  { id: "outputs", label: "Outputs" },
];

function statusTone(
  status: string,
): "active" | "complete" | "warning" | "muted" {
  if (["running", "active", "in_progress"].includes(status)) return "active";
  if (["completed", "complete", "executed", "approved"].includes(status))
    return "complete";
  if (["blocked", "failed", "interrupted"].includes(status)) return "warning";
  return "muted";
}

function familyRecords(
  map: SessionMap,
  family: SessionMapFamily,
): SessionMapResource[] {
  switch (family) {
    case "delegates":
      return map.delegates.map((value) => ({ family, value }));
    case "goals":
      return map.goals.map((value) => ({ family, value }));
    case "tasks":
      return map.tasks.map((value) => ({ family, value }));
    case "plans":
      return map.plans.map((value) => ({ family, value }));
    case "decisions":
      return map.decisions.map((value) => ({ family, value }));
    case "memories":
      return map.memories.map((value) => ({ family, value }));
    case "research":
      return map.researchRuns.map((value) => ({ family, value }));
    case "sources":
      return map.researchSources.map((value) => ({ family, value }));
    case "artifacts":
      return [];
  }
}

function recordTitle(record: SessionMapResource): string {
  switch (record.family) {
    case "delegates":
      return record.value.task;
    case "goals":
      return record.value.objective;
    case "tasks":
      return record.value.title;
    case "plans":
      return record.value.prompt;
    case "decisions":
      return record.value.title;
    case "memories":
      return record.value.text;
    case "research":
      return record.value.question;
    case "sources":
      return record.value.title;
  }
}

function recordStatus(record: SessionMapResource): string {
  return record.family === "sources" ? record.value.kind : record.value.status;
}

function recordId(record: SessionMapResource): string {
  return record.family === "delegates" ? record.value.jobId : record.value.id;
}

function recordMeta(
  record: SessionMapResource,
  showRunLineage: boolean,
): string {
  switch (record.family) {
    case "delegates":
      return `${record.value.role}${showRunLineage ? ` · ${record.value.parentRunId}` : ""}`;
    case "goals":
      return `${record.value.iterationsCompleted} / ${record.value.iterationBudget} iterations`;
    case "tasks":
      return record.value.description;
    case "plans":
      return `Revision ${record.value.revision} · ${record.value.stepCount} steps`;
    case "decisions":
      return `${record.value.priority} · ${record.value.source}`;
    case "memories":
      return `${record.value.scope} · ${record.value.kind}`;
    case "research":
      return `${record.value.depth} · ${record.value.sourceCount} sources`;
    case "sources":
      return `${record.value.label} · ${record.value.kind}`;
  }
}

export function SessionTopology({
  views,
  participants,
  sessionMap,
  loading,
  error,
  artifacts,
  onSelectResource,
  onSelectArtifact,
}: {
  views: readonly RunView[];
  participants: readonly AgentParticipant[];
  sessionMap: SessionMap | null;
  loading: boolean;
  error: string;
  artifacts: readonly ArtifactViewItem[];
  onSelectResource: (resource: SessionMapResource) => void;
  onSelectArtifact: (artifactId: string) => void;
}) {
  const [expanded, setExpanded] = useState<ReadonlySet<SessionMapFamily>>(
    () => new Set(["memories"]),
  );
  const [layers, setLayers] = useState<ReadonlySet<SessionMapLayer>>(
    () => new Set(LAYER_META.map(({ id }) => id)),
  );
  const [showRunLineage, setShowRunLineage] = useState(false);
  const stageRef = useRef<HTMLDivElement>(null);
  const primary = participants.find(({ kind }) => kind === "primary");
  const opening = views[0];
  const visibleFamilies = useMemo(
    () => FAMILY_META.filter(({ layer }) => layers.has(layer)),
    [layers],
  );

  function countFor(family: SessionMapFamily): number {
    if (family === "artifacts") return artifacts.length;
    return sessionMap === null ? 0 : familyRecords(sessionMap, family).length;
  }

  function toggleFamily(family: SessionMapFamily) {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(family)) next.delete(family);
      else next.add(family);
      return next;
    });
  }

  return (
    <section className="session-map" aria-labelledby="session-topology-title">
      <header className="session-map-header">
        <div>
          <h3 id="session-topology-title">Session map</h3>
          <p>Primary session · canonical resources and agent lineage</p>
        </div>
        <div className="session-map-actions">
          <button
            type="button"
            onClick={() => {
              setExpanded(new Set(["memories"]));
              stageRef.current?.scrollTo({ top: 0, left: 0 });
            }}
          >
            <IconArrowsMaximize size={15} stroke={1.6} /> Fit
          </button>
          <button type="button" onClick={() => setExpanded(new Set())}>
            <IconListCheck size={15} stroke={1.6} /> Collapse all
          </button>
          <label>
            <span className="sr-only">Session map scope</span>
            <select defaultValue="session">
              <option value="session">Entire session</option>
            </select>
          </label>
        </div>
      </header>

      <div ref={stageRef} className="session-map-stage">
        <aside className="session-map-layers" aria-label="Session map layers">
          <header>
            <IconAdjustments size={15} stroke={1.6} />
            <strong>Layers</strong>
          </header>
          {LAYER_META.map((layer) => (
            <label key={layer.id}>
              <span>{layer.label}</span>
              <input
                type="checkbox"
                checked={layers.has(layer.id)}
                onChange={() =>
                  setLayers((current) => {
                    const next = new Set(current);
                    if (next.has(layer.id)) next.delete(layer.id);
                    else next.add(layer.id);
                    return next;
                  })
                }
              />
            </label>
          ))}
        </aside>

        {loading ? (
          <div className="session-map-loading" role="status">
            <span /> Loading session resources…
          </div>
        ) : sessionMap === null ||
          opening === undefined ||
          primary === undefined ? (
          <div className="session-view-empty">
            <IconTopologyStar3 size={26} stroke={1.5} aria-hidden="true" />
            <strong>Session map unavailable</strong>
            <span>
              {error ||
                "Canonical session resources will appear here when released."}
            </span>
          </div>
        ) : (
          <div
            className="session-map-network"
            style={
              { "--map-row-count": visibleFamilies.length } as CSSProperties
            }
          >
            <article className="session-map-primary">
              <span aria-hidden="true">
                <IconSparkles size={20} stroke={1.65} />
              </span>
              <div>
                <strong>{primary.name}</strong>
                <small>Started {shortDateLabel(opening.run.createdAt)}</small>
              </div>
              <em>
                <i /> {readableState(primary.state)}
              </em>
            </article>
            <div className="session-map-trunk" aria-hidden="true" />
            {visibleFamilies.map((family, index) => {
              const Icon = family.icon;
              const records =
                family.id === "artifacts"
                  ? []
                  : familyRecords(sessionMap, family.id);
              const open = expanded.has(family.id);
              const count = countFor(family.id);
              return (
                <div
                  className="session-map-row"
                  style={{ "--map-row": index + 1 } as CSSProperties}
                  key={family.id}
                >
                  <button
                    className={`session-map-family family-${family.layer}`}
                    type="button"
                    aria-expanded={open}
                    onClick={() => toggleFamily(family.id)}
                  >
                    <span aria-hidden="true">
                      <Icon size={18} stroke={1.6} />
                    </span>
                    <span>
                      <strong>
                        {family.label} <b>{count}</b>
                      </strong>
                      <small>
                        {count === 0
                          ? "No released records"
                          : `${count} released`}
                      </small>
                    </span>
                    <IconChevronDown
                      size={15}
                      stroke={1.6}
                      aria-hidden="true"
                    />
                  </button>
                  {open ? (
                    <ol className="session-map-children">
                      {family.id === "artifacts"
                        ? artifacts.map((artifact) => (
                            <li key={artifact.id}>
                              <button
                                type="button"
                                onClick={() => onSelectArtifact(artifact.id)}
                              >
                                <span>
                                  <strong>{artifact.fileName}</strong>
                                  <small>
                                    {artifact.mediaType} · {artifact.sizeLabel}
                                  </small>
                                </span>
                                <IconChevronRight size={15} />
                              </button>
                            </li>
                          ))
                        : records.map((record) => {
                            const status = recordStatus(record);
                            return (
                              <li key={`${record.family}:${recordId(record)}`}>
                                <button
                                  type="button"
                                  onClick={() => onSelectResource(record)}
                                >
                                  <span>
                                    <strong>{recordTitle(record)}</strong>
                                    <small>
                                      {recordMeta(record, showRunLineage)}
                                    </small>
                                  </span>
                                  <em className={`tone-${statusTone(status)}`}>
                                    <i /> {status.replaceAll("_", " ")}
                                  </em>
                                  <IconChevronRight
                                    size={15}
                                    stroke={1.6}
                                    aria-hidden="true"
                                  />
                                </button>
                              </li>
                            );
                          })}
                      {count === 0 ? (
                        <li className="session-map-child-empty">
                          No released records in this family.
                        </li>
                      ) : null}
                    </ol>
                  ) : null}
                </div>
              );
            })}
          </div>
        )}
      </div>
      <label className="session-map-lineage-toggle">
        <input
          type="checkbox"
          checked={showRunLineage}
          onChange={(event) => setShowRunLineage(event.currentTarget.checked)}
        />
        <span>Show run lineage</span>
      </label>
      {error !== "" && sessionMap !== null ? (
        <p className="session-map-stale-note">{error}</p>
      ) : null}
    </section>
  );
}

function planStatus(plan: SessionPlanReference): string {
  if (plan.cancelled) {
    return "Saved before cancellation";
  }
  return plan.status === null
    ? "Draft"
    : plan.status.charAt(0).toUpperCase() + plan.status.slice(1);
}

export function SessionPlansView({
  views,
  workflowAvailable,
  onInspectPlan,
  onOpenPlanWorkflow,
  onRevisePlan,
}: {
  views: readonly RunView[];
  workflowAvailable: boolean;
  onInspectPlan: (plan: SessionPlanReference) => void;
  onOpenPlanWorkflow: (sessionId: string, planId: string) => void;
  onRevisePlan: (sourceRunId: string, planId: string, revision: number) => void;
}) {
  const plans = selectSessionPlans(views);
  return (
    <section
      className="session-resource-view"
      aria-labelledby="session-plans-title"
    >
      <header className="session-view-summary">
        <div>
          <h3 id="session-plans-title">Plans</h3>
          <p>Durable plans released by runs in this session.</p>
        </div>
        <span>{plans.length}</span>
      </header>
      {plans.length === 0 ? (
        <div className="session-view-empty">
          <IconListDetails size={26} stroke={1.5} aria-hidden="true" />
          <strong>No plans in this session</strong>
          <span>Plan-mode results will appear here when released.</span>
        </div>
      ) : (
        <ol className="session-plan-list">
          {plans.map((plan) => (
            <li key={plan.planId}>
              <article>
                <span className="session-resource-icon" aria-hidden="true">
                  <IconListDetails size={18} stroke={1.6} />
                </span>
                <div>
                  <strong>{plan.sourceRunTitle}</strong>
                  <small>
                    Run {plan.runIndex} · Revision {plan.revision} ·{" "}
                    {planStatus(plan)}
                  </small>
                  <p>
                    {plan.output ||
                      "The plan was saved without a released preview."}
                  </p>
                </div>
                <div className="session-resource-actions">
                  <button type="button" onClick={() => onInspectPlan(plan)}>
                    Read plan
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      onRevisePlan(plan.sourceRunId, plan.planId, plan.revision)
                    }
                  >
                    Revise
                  </button>
                  <button
                    type="button"
                    disabled={!workflowAvailable}
                    onClick={() =>
                      onOpenPlanWorkflow(
                        views[0]?.run.sessionId ?? "",
                        plan.planId,
                      )
                    }
                  >
                    Open workflow
                  </button>
                </div>
              </article>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

export function SessionSourcesView({
  views,
  onOpenWorkspaceFile,
}: {
  views: readonly RunView[];
  onOpenWorkspaceFile: (path: string) => void;
}) {
  const sources = selectSessionSources(views);
  return (
    <section
      className="session-resource-view"
      aria-labelledby="session-sources-title"
    >
      <header className="session-view-summary">
        <div>
          <h3 id="session-sources-title">Sources</h3>
          <p>Released web and workspace citations across this session.</p>
        </div>
        <span>{sources.length}</span>
      </header>
      {sources.length === 0 ? (
        <div className="session-view-empty">
          <IconBook2 size={26} stroke={1.5} aria-hidden="true" />
          <strong>No released sources</strong>
          <span>
            Research citations will remain available here across follow-ups.
          </span>
        </div>
      ) : (
        <ol className="session-source-list">
          {sources.map((source) => {
            const workspacePath = workspaceSourcePath(source.uri);
            return (
              <li key={`${source.label}:${source.uri}`}>
                <span className="session-resource-icon" aria-hidden="true">
                  {isWebUri(source.uri) ? (
                    <IconExternalLink size={17} stroke={1.6} />
                  ) : (
                    <IconFileText size={17} stroke={1.6} />
                  )}
                </span>
                <span>
                  <small>
                    {source.label} · {source.sourceRunTitle}
                  </small>
                  <strong>{source.title}</strong>
                  {isWebUri(source.uri) ? (
                    <a href={source.uri} target="_blank" rel="noreferrer">
                      {source.uri}
                    </a>
                  ) : workspacePath !== null ? (
                    <button
                      type="button"
                      onClick={() => onOpenWorkspaceFile(workspacePath)}
                    >
                      {workspacePath} · Open file
                    </button>
                  ) : (
                    <code>{source.uri}</code>
                  )}
                </span>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}

export function SessionResourcesView({
  views,
  artifacts,
  onChangeView,
  onSelectArtifact,
}: {
  views: readonly RunView[];
  artifacts: readonly ArtifactViewItem[];
  onChangeView: (view: SessionWorkspaceView) => void;
  onSelectArtifact: (artifactId: string) => void;
}) {
  const plans = selectSessionPlans(views);
  const sources = selectSessionSources(views);
  return (
    <section
      className="session-resource-view"
      aria-labelledby="session-resources-title"
    >
      <header className="session-view-summary">
        <div>
          <h3 id="session-resources-title">Resources</h3>
          <p>Released records and files associated with this session.</p>
        </div>
      </header>
      <div className="session-resource-groups">
        <button type="button" onClick={() => onChangeView("plans")}>
          <IconListDetails size={18} stroke={1.6} aria-hidden="true" />
          <span>
            <strong>Plans</strong>
            <small>Durable session plans</small>
          </span>
          <b>{plans.length}</b>
          <IconChevronRight size={15} stroke={1.6} aria-hidden="true" />
        </button>
        <button type="button" onClick={() => onChangeView("sources")}>
          <IconBook2 size={18} stroke={1.6} aria-hidden="true" />
          <span>
            <strong>Sources</strong>
            <small>Released research evidence</small>
          </span>
          <b>{sources.length}</b>
          <IconChevronRight size={15} stroke={1.6} aria-hidden="true" />
        </button>
        <div className="session-artifact-group">
          <header>
            <IconFolder size={18} stroke={1.6} aria-hidden="true" />
            <span>
              <strong>Artifacts</strong>
              <small>Files released by this session</small>
            </span>
            <b>{artifacts.length}</b>
          </header>
          {artifacts.length === 0 ? (
            <p>No artifacts have been released.</p>
          ) : (
            <ol>
              {artifacts.map((artifact) => (
                <li key={artifact.id}>
                  <button
                    type="button"
                    onClick={() => onSelectArtifact(artifact.id)}
                  >
                    <IconFileText size={15} stroke={1.6} aria-hidden="true" />
                    <span>
                      <strong>{artifact.fileName}</strong>
                      <small>{artifact.stateLabel}</small>
                    </span>
                    <IconChevronRight
                      size={14}
                      stroke={1.6}
                      aria-hidden="true"
                    />
                  </button>
                </li>
              ))}
            </ol>
          )}
        </div>
        <div className="session-resource-unavailable">
          <IconMessageCircle size={18} stroke={1.6} aria-hidden="true" />
          <span>
            <strong>Decisions</strong>
            <small>No released decisions in this session</small>
          </span>
          <IconCheck size={15} stroke={1.7} aria-hidden="true" />
        </div>
      </div>
    </section>
  );
}
