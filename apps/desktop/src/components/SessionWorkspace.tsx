import {
  IconAdjustments,
  IconArrowsMaximize,
  IconBook2,
  IconChecklist,
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
  IconTargetArrow,
  IconTopologyStar3,
} from "@tabler/icons-react";
import { lazy, Suspense, useCallback, useMemo, useState } from "react";
import type { ComponentType } from "react";

import { shortDateLabel } from "../presenters";
import { selectSessionPlans, selectSessionSources } from "../session-resources";
import type { SessionPlanReference } from "../session-resources";
import type { RunView } from "../state";
import type { SessionMap, SessionMapResource } from "../types";
import type { AgentParticipant, AgentWorkState } from "./AgentFlow";
import type { ArtifactViewItem } from "./ArtifactWorkspace";
import { DropdownSelect } from "./DropdownSelect";
import { MarkdownContent } from "./MarkdownContent";
import { isWebUri, workspaceSourcePath } from "./ResearchSourcesPanel";
import type {
  SessionTopologyFamilyModel,
  SessionTopologyPrimaryModel,
} from "./SessionTopologyGraph";

const SessionTopologyGraph = lazy(() =>
  import("./SessionTopologyGraph").then(({ SessionTopologyGraph }) => ({
    default: SessionTopologyGraph,
  })),
);

export type SessionWorkspaceView =
  | "conversation"
  | "topology"
  | "plans"
  | "snapshots"
  | "activity"
  | "sources"
  | "resources";

const SESSION_TABS: ReadonlyArray<{
  id: SessionWorkspaceView;
  label: string;
}> = [
  { id: "conversation", label: "Conversation" },
  { id: "topology", label: "Topology" },
  { id: "plans", label: "Plans" },
  { id: "snapshots", label: "Snapshots" },
  { id: "activity", label: "Activity" },
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
  | "snapshots"
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
  {
    id: "snapshots",
    label: "Context snapshots",
    layer: "context",
    icon: IconAdjustments,
  },
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
    case "snapshots":
      return map.contextSnapshots.map((value) => ({ family, value }));
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
    case "snapshots":
      return (
        record.value.summary ||
        `Messages ${record.value.sourceStartSequence}–${record.value.sourceEndSequence}`
      );
    case "research":
      return record.value.question;
    case "sources":
      return record.value.title;
  }
}

function recordStatus(record: SessionMapResource): string {
  if (record.family === "sources") return record.value.kind;
  if (record.family === "snapshots") return "immutable";
  return record.value.status;
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
    case "snapshots":
      return `${record.value.strategy.replaceAll("_", " ")} · messages ${record.value.sourceStartSequence}–${record.value.sourceEndSequence}`;
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
  const [fitRequest, setFitRequest] = useState(0);
  const primary = participants.find(({ kind }) => kind === "primary");
  const opening = views[0];
  const visibleFamilies = useMemo(
    () => FAMILY_META.filter(({ layer }) => layers.has(layer)),
    [layers],
  );

  const toggleFamily = useCallback((family: SessionMapFamily) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(family)) next.delete(family);
      else next.add(family);
      return next;
    });
  }, []);

  const graphPrimary = useMemo<SessionTopologyPrimaryModel | null>(() => {
    if (primary === undefined || opening === undefined) return null;
    return {
      name: primary.name,
      startedLabel: shortDateLabel(opening.run.createdAt),
      stateLabel: readableState(primary.state),
    };
  }, [opening, primary]);

  const graphFamilies = useMemo<readonly SessionTopologyFamilyModel[]>(() => {
    if (sessionMap === null) return [];
    return visibleFamilies.map((family) => {
      const records =
        family.id === "artifacts"
          ? artifacts.map((artifact) => ({
              id: artifact.id,
              title: artifact.fileName,
              meta: `${artifact.mediaType} · ${artifact.sizeLabel}`,
              statusLabel: artifact.stateLabel,
              tone: statusTone(
                artifact.stateLabel.toLowerCase().replaceAll(" ", "_"),
              ),
              onSelect: () => onSelectArtifact(artifact.id),
            }))
          : familyRecords(sessionMap, family.id).map((record) => {
              const status = recordStatus(record);
              return {
                id: recordId(record),
                title: recordTitle(record),
                meta: recordMeta(record, showRunLineage),
                statusLabel: status.replaceAll("_", " "),
                tone: statusTone(status),
                onSelect: () => onSelectResource(record),
              };
            });
      return {
        ...family,
        count: records.length,
        open: expanded.has(family.id),
        records,
        onToggle: () => toggleFamily(family.id),
      };
    });
  }, [
    artifacts,
    expanded,
    onSelectArtifact,
    onSelectResource,
    sessionMap,
    showRunLineage,
    toggleFamily,
    visibleFamilies,
  ]);

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
              setFitRequest((current) => current + 1);
            }}
          >
            <IconArrowsMaximize size={15} stroke={1.6} /> Fit
          </button>
          <button type="button" onClick={() => setExpanded(new Set())}>
            <IconListCheck size={15} stroke={1.6} /> Collapse all
          </button>
          <label>
            <span className="sr-only">Session map scope</span>
            <DropdownSelect defaultValue="session">
              <option value="session">Entire session</option>
            </DropdownSelect>
          </label>
        </div>
      </header>

      <div className="session-map-stage">
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
                role="switch"
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
          primary === undefined ||
          graphPrimary === null ? (
          <div className="session-view-empty">
            <IconTopologyStar3 size={26} stroke={1.5} aria-hidden="true" />
            <strong>Session map unavailable</strong>
            <span>
              {error ||
                "Canonical session resources will appear here when released."}
            </span>
          </div>
        ) : (
          <Suspense
            fallback={
              <div className="session-map-loading" role="status">
                <span /> Loading interactive session map…
              </div>
            }
          >
            <SessionTopologyGraph
              primary={graphPrimary}
              families={graphFamilies}
              fitRequest={fitRequest}
            />
          </Suspense>
        )}
        <label className="session-map-lineage-toggle">
          <input
            type="checkbox"
            role="switch"
            checked={showRunLineage}
            onChange={(event) => setShowRunLineage(event.currentTarget.checked)}
          />
          <span>Show run lineage</span>
        </label>
      </div>
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
                  <button
                    className="session-plan-title"
                    type="button"
                    onClick={() => onInspectPlan(plan)}
                  >
                    {plan.sourceRunTitle}
                  </button>
                  <small>
                    Run {plan.runIndex} · Revision {plan.revision} ·{" "}
                    {planStatus(plan)}
                  </small>
                  {plan.output.trim() === "" ? (
                    <p className="session-plan-preview-fallback">
                      The plan was saved without a released preview.
                    </p>
                  ) : (
                    <MarkdownContent
                      className="session-plan-preview"
                      content={plan.output}
                    />
                  )}
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

export function SessionSnapshotsView({
  sessionMap,
  loading,
  error,
  onSelectResource,
}: {
  sessionMap: SessionMap | null;
  loading: boolean;
  error: string;
  onSelectResource: (resource: SessionMapResource) => void;
}) {
  const snapshots = sessionMap?.contextSnapshots ?? [];
  return (
    <section
      className="session-resource-view"
      aria-labelledby="session-snapshots-title"
    >
      <header className="session-view-summary">
        <div>
          <h3 id="session-snapshots-title">Context snapshots</h3>
          <p>
            Immutable summaries used to compact model context without deleting
            canonical conversation history.
          </p>
        </div>
        <span>{snapshots.length}</span>
      </header>
      {loading && sessionMap === null ? (
        <div className="session-view-empty">
          <IconAdjustments size={26} stroke={1.5} aria-hidden="true" />
          <strong>Loading context snapshots</strong>
          <span>Reading bounded records from the selected session.</span>
        </div>
      ) : error !== "" && sessionMap === null ? (
        <div className="session-view-empty is-error" role="alert">
          <IconAdjustments size={26} stroke={1.5} aria-hidden="true" />
          <strong>Context snapshots are unavailable</strong>
          <span>{error}</span>
        </div>
      ) : snapshots.length === 0 ? (
        <div className="session-view-empty">
          <IconAdjustments size={26} stroke={1.5} aria-hidden="true" />
          <strong>No context snapshots</strong>
          <span>
            Automatic or manual context compaction will create immutable
            snapshots here.
          </span>
        </div>
      ) : (
        <ol className="session-snapshot-list">
          {snapshots.map((snapshot) => {
            const resource: SessionMapResource = {
              family: "snapshots",
              value: snapshot,
            };
            return (
              <li key={snapshot.id}>
                <article>
                  <span className="session-resource-icon" aria-hidden="true">
                    <IconAdjustments size={18} stroke={1.6} />
                  </span>
                  <div>
                    <button
                      className="session-plan-title"
                      type="button"
                      onClick={() => onSelectResource(resource)}
                    >
                      Messages {snapshot.sourceStartSequence}–
                      {snapshot.sourceEndSequence}
                    </button>
                    <small>
                      {snapshot.strategy.replaceAll("_", " ")} · {snapshot.id}
                    </small>
                    <p>{snapshot.summary || "No summary was recorded."}</p>
                    <span className="session-snapshot-counts">
                      {snapshot.pinnedFacts.length} facts ·{" "}
                      {snapshot.openTasks.length} open tasks ·{" "}
                      {snapshot.filesTouched.length} files
                    </span>
                  </div>
                  <div className="session-resource-actions">
                    <button
                      type="button"
                      onClick={() => onSelectResource(resource)}
                    >
                      View snapshot
                    </button>
                  </div>
                </article>
              </li>
            );
          })}
        </ol>
      )}
      {error !== "" && sessionMap !== null ? (
        <p className="session-map-stale-note">{error}</p>
      ) : null}
    </section>
  );
}

const RESOURCE_RECORD_FAMILIES = FAMILY_META.filter(
  ({ id }) => !["artifacts", "plans", "sources", "snapshots"].includes(id),
);

function SessionRecordGroup({
  map,
  family,
  onSelectResource,
}: {
  map: SessionMap;
  family: (typeof RESOURCE_RECORD_FAMILIES)[number];
  onSelectResource: (resource: SessionMapResource) => void;
}) {
  const records = familyRecords(map, family.id);
  const Icon = family.icon;
  return (
    <details className="session-record-group">
      <summary>
        <Icon size={18} stroke={1.6} aria-hidden="true" />
        <span>
          <strong>{family.label}</strong>
          <small>Durable records available in this session</small>
        </span>
        <b>{records.length}</b>
        <IconChevronRight size={15} stroke={1.6} aria-hidden="true" />
      </summary>
      {records.length === 0 ? (
        <p>No {family.label.toLocaleLowerCase()} are stored.</p>
      ) : (
        <ol>
          {records.map((record) => (
            <li key={recordId(record)}>
              <button type="button" onClick={() => onSelectResource(record)}>
                <span>
                  <strong>{recordTitle(record)}</strong>
                  <small>
                    {recordMeta(record, false)} ·{" "}
                    {recordStatus(record).replaceAll("_", " ")}
                  </small>
                </span>
                <IconChevronRight size={14} stroke={1.6} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ol>
      )}
    </details>
  );
}

export function SessionResourcesView({
  views,
  artifacts,
  sessionMap,
  loading,
  error,
  onChangeView,
  onSelectArtifact,
  onSelectResource,
}: {
  views: readonly RunView[];
  artifacts: readonly ArtifactViewItem[];
  sessionMap: SessionMap | null;
  loading: boolean;
  error: string;
  onChangeView: (view: SessionWorkspaceView) => void;
  onSelectArtifact: (artifactId: string) => void;
  onSelectResource: (resource: SessionMapResource) => void;
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
        <button type="button" onClick={() => onChangeView("snapshots")}>
          <IconAdjustments size={18} stroke={1.6} aria-hidden="true" />
          <span>
            <strong>Context snapshots</strong>
            <small>Immutable compacted context</small>
          </span>
          <b>{sessionMap?.contextSnapshots.length ?? 0}</b>
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
        {loading && sessionMap === null ? (
          <div className="session-resource-unavailable">
            <IconMessageCircle size={18} stroke={1.6} aria-hidden="true" />
            <span>
              <strong>Loading durable records</strong>
              <small>Reading the selected session map</small>
            </span>
          </div>
        ) : null}
        {error !== "" && sessionMap === null ? (
          <div className="session-resource-unavailable is-error" role="alert">
            <IconMessageCircle size={18} stroke={1.6} aria-hidden="true" />
            <span>
              <strong>Durable records are unavailable</strong>
              <small>{error}</small>
            </span>
          </div>
        ) : null}
        {sessionMap === null
          ? null
          : RESOURCE_RECORD_FAMILIES.map((family) => (
              <SessionRecordGroup
                key={family.id}
                map={sessionMap}
                family={family}
                onSelectResource={onSelectResource}
              />
            ))}
      </div>
    </section>
  );
}
