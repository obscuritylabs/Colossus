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
import type { ComponentType, ReactNode } from "react";

import { shortDateLabel } from "./types";
import {
  canContinuePlan,
  selectSessionPlans,
  selectSessionSources,
} from "./selectors";
import type { SessionPlanReference } from "./selectors";
import type { RunView } from "./types";
import type { SessionMap, SessionMapResource } from "./types";
import type { AgentParticipant, AgentWorkState } from "./types";
import type { ArtifactViewItem } from "./types";
import { MarkdownContent, SessionLink } from "./links";
import { Button } from "../components/ui/button";
import { Badge } from "../components/ui/badge";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "../components/ui/popover";
import {
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
  EmptyDescription,
} from "../components/ui/empty";
import {
  Item,
  ItemMedia,
  ItemContent,
  ItemTitle,
  ItemDescription,
  ItemActions,
} from "../components/ui/item";
import { isWebUri, workspaceSourcePath } from "./sources";
import type {
  SessionTopologyFamilyModel,
  SessionTopologyPrimaryModel,
} from "./SessionTopologyGraph";

const SessionTopologyGraph = lazy(() =>
  import("./SessionTopologyGraph").then(({ SessionTopologyGraph }) => ({
    default: SessionTopologyGraph,
  })),
);

export { SessionWorkspaceTabs } from "../conversation";
import type { SessionWorkspaceView } from "../conversation";
export type { SessionWorkspaceView } from "../conversation";

function SessionEmpty({
  icon: Icon,
  title,
  children,
  loading = false,
  alert = false,
}: {
  icon: ComponentType<{ size?: number; stroke?: number }>;
  title: string;
  children: ReactNode;
  loading?: boolean;
  alert?: boolean;
}) {
  return (
    <Empty
      className="session-view-empty"
      role={alert ? "alert" : loading ? "status" : undefined}
    >
      <EmptyHeader>
        <EmptyMedia variant="icon" aria-hidden="true">
          <Icon size={20} stroke={1.6} />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{children}</EmptyDescription>
      </EmptyHeader>
    </Empty>
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
      return `Messages ${record.value.sourceStartSequence}–${record.value.sourceEndSequence}`;
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

  if (
    loading ||
    sessionMap === null ||
    graphPrimary === null ||
    sessionMap.sessionId !== opening?.run.sessionId
  ) {
    return (
      <section
        className="session-resource-view"
        aria-labelledby="session-topology-title"
      >
        <header className="session-view-summary">
          <div>
            <h3 id="session-topology-title">Session map</h3>
            <p>Explore the resources released by this session.</p>
          </div>
        </header>
        <SessionEmpty
          icon={IconTopologyStar3}
          title={
            loading ? "Loading session resources" : "Session map unavailable"
          }
          loading={loading}
        >
          {loading
            ? "Reading records from the selected session."
            : error || "Session resources will appear here when released."}
        </SessionEmpty>
      </section>
    );
  }

  return (
    <section className="session-map" aria-labelledby="session-topology-title">
      <header className="session-map-header">
        <div>
          <h3 id="session-topology-title">Session map</h3>
          <p>Primary session · canonical resources and agent lineage</p>
        </div>
        <div className="session-map-actions">
          <Button
            variant="outline"
            onClick={() => {
              setExpanded(new Set(["memories"]));
              setFitRequest((current) => current + 1);
            }}
          >
            <IconArrowsMaximize size={15} stroke={1.6} /> Fit
          </Button>
          <Button variant="outline" onClick={() => setExpanded(new Set())}>
            <IconListCheck size={15} stroke={1.6} /> Collapse all
          </Button>
          <Popover>
            <PopoverTrigger asChild>
              <Button variant="outline">
                <IconAdjustments size={15} aria-hidden="true" /> Layers
              </Button>
            </PopoverTrigger>
            <PopoverContent
              className="session-map-layers"
              aria-label="Session map layers"
              align="end"
            >
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
            </PopoverContent>
          </Popover>
        </div>
      </header>

      <div className="session-map-stage">
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
        <label className="session-map-lineage-toggle">
          <input
            type="checkbox"
            role="switch"
            checked={showRunLineage}
            onChange={(event) => setShowRunLineage(event.currentTarget.checked)}
          />
          <span>Show parent run IDs</span>
        </label>
      </div>
      {error !== "" && sessionMap !== null ? (
        <p className="session-map-stale-note" role="status">
          {error}
        </p>
      ) : null}
    </section>
  );
}

function planStatus(plan: SessionPlanReference): string {
  if (plan.cancelled) {
    return "Saved before cancellation";
  }
  return plan.status === null
    ? "Status not reported"
    : plan.status.charAt(0).toUpperCase() + plan.status.slice(1);
}

export function SessionPlansView({
  views,
  continuationAvailable,
  workflowAvailable,
  onInspectPlan,
  onOpenPlanWorkflow,
  onRevisePlan,
}: {
  views: readonly RunView[];
  continuationAvailable: boolean;
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
          <p>Plans released by loaded runs in this session.</p>
        </div>
        <Badge
          className="session-view-count"
          aria-label={`${plans.length} plans in loaded runs`}
        >
          {plans.length}
        </Badge>
      </header>
      {plans.length === 0 ? (
        <SessionEmpty icon={IconListDetails} title="No plans in this session">
          Plan-mode results will appear here when released.
        </SessionEmpty>
      ) : (
        <ol className="session-plan-list">
          {plans.map((plan) => (
            <li key={plan.planId}>
              <article>
                <span className="session-resource-icon" aria-hidden="true">
                  <IconListDetails size={18} stroke={1.6} />
                </span>
                <div>
                  <Button
                    className="session-plan-title"
                    variant="link"
                    onClick={() => onInspectPlan(plan)}
                  >
                    {plan.sourceRunTitle}
                  </Button>
                  <small>
                    Run {plan.runIndex} · Revision {plan.revision} ·{" "}
                    <Badge>{planStatus(plan)}</Badge>
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
                  <Button variant="outline" onClick={() => onInspectPlan(plan)}>
                    Read plan
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={!canContinuePlan(plan, continuationAvailable)}
                    onClick={() =>
                      onRevisePlan(plan.sourceRunId, plan.planId, plan.revision)
                    }
                  >
                    Revise
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={!workflowAvailable}
                    onClick={() =>
                      onOpenPlanWorkflow(
                        views[0]?.run.sessionId ?? "",
                        plan.planId,
                      )
                    }
                  >
                    Open workflow
                  </Button>
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
  onOpenWorkspaceFile?: (path: string) => void;
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
          <p>Citations from loaded research results in this session.</p>
        </div>
        <Badge
          className="session-view-count"
          aria-label={`${sources.length} sources in loaded results`}
        >
          {sources.length}
        </Badge>
      </header>
      {sources.length === 0 ? (
        <SessionEmpty icon={IconBook2} title="No released sources">
          Research citations will appear here when released.
        </SessionEmpty>
      ) : (
        <ol className="session-source-list">
          {sources.map((source) => {
            const workspacePath = workspaceSourcePath(source.uri);
            return (
              <li key={`${source.label}:${source.uri}`}>
                <Item>
                  <ItemMedia variant="icon" aria-hidden="true">
                    {isWebUri(source.uri) ? (
                      <IconExternalLink size={17} stroke={1.6} />
                    ) : (
                      <IconFileText size={17} stroke={1.6} />
                    )}
                  </ItemMedia>
                  <ItemContent className="session-source-copy">
                    <ItemTitle>{source.title}</ItemTitle>
                    <ItemDescription>
                      <Badge>{source.label}</Badge> {source.sourceRunTitle}
                    </ItemDescription>
                    <div className="session-source-location">
                      {isWebUri(source.uri) ? (
                        <SessionLink href={source.uri}>
                          {source.uri}
                        </SessionLink>
                      ) : (
                        <code>{source.uri}</code>
                      )}
                    </div>
                  </ItemContent>
                  {workspacePath !== null && onOpenWorkspaceFile ? (
                    <ItemActions>
                      <Button
                        variant="outline"
                        onClick={() => onOpenWorkspaceFile(workspacePath)}
                      >
                        Open file
                      </Button>
                    </ItemActions>
                  ) : null}
                </Item>
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
        <Badge
          className="session-view-count"
          aria-label={
            sessionMap === null
              ? "Snapshot count unavailable"
              : `${snapshots.length} snapshots`
          }
        >
          {sessionMap === null ? "—" : snapshots.length}
        </Badge>
      </header>
      {loading && sessionMap === null ? (
        <SessionEmpty
          icon={IconAdjustments}
          title="Loading context snapshots"
          loading
        >
          Reading bounded records from the selected session.
        </SessionEmpty>
      ) : error !== "" && sessionMap === null ? (
        <SessionEmpty
          icon={IconAdjustments}
          title="Context snapshots are unavailable"
          alert
        >
          {error}
        </SessionEmpty>
      ) : sessionMap === null ? (
        <SessionEmpty
          icon={IconAdjustments}
          title="Context snapshots are unavailable"
        >
          This connection has not released a session map.
        </SessionEmpty>
      ) : snapshots.length === 0 ? (
        <SessionEmpty icon={IconAdjustments} title="No context snapshots">
          Automatic or manual context compaction will create immutable snapshots
          here.
        </SessionEmpty>
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
                    <Button
                      className="session-plan-title"
                      variant="link"
                      onClick={() => onSelectResource(resource)}
                    >
                      Messages {snapshot.sourceStartSequence}–
                      {snapshot.sourceEndSequence}
                    </Button>
                    <small>
                      {snapshot.strategy.replaceAll("_", " ")} · {snapshot.id}
                    </small>
                    <MarkdownContent
                      className="session-snapshot-preview"
                      content={snapshot.summary || "No summary was recorded."}
                    />
                    <span className="session-snapshot-counts">
                      {snapshot.pinnedFacts.length} facts ·{" "}
                      {snapshot.openTasks.length} open tasks ·{" "}
                      {snapshot.filesTouched.length} files
                    </span>
                  </div>
                  <div className="session-resource-actions">
                    <Button
                      variant="outline"
                      onClick={() => onSelectResource(resource)}
                    >
                      View snapshot
                    </Button>
                  </div>
                </article>
              </li>
            );
          })}
        </ol>
      )}
      {error !== "" && sessionMap !== null ? (
        <p className="session-map-stale-note" role="status">
          {error}
        </p>
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
          <small>
            {family.id === "decisions"
              ? "Active and historical decisions across this Workspace"
              : "Durable records available in this session"}
          </small>
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
  artifactCoverage,
  sessionMap,
  loading,
  error,
  onChangeView,
  onSelectArtifact,
  onSelectResource,
}: {
  views: readonly RunView[];
  artifacts: readonly ArtifactViewItem[];
  artifactCoverage?: string;
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
        {(
          [
            {
              view: "plans",
              title: "Plans",
              description: "Plans from loaded runs",
              count: plans.length,
              Icon: IconListDetails,
            },
            {
              view: "sources",
              title: "Sources",
              description: "Citations from loaded results",
              count: sources.length,
              Icon: IconBook2,
            },
            {
              view: "snapshots",
              title: "Context snapshots",
              description: "Immutable compacted context",
              count: sessionMap?.contextSnapshots.length ?? "—",
              Icon: IconAdjustments,
            },
          ] as const
        ).map(({ view, title, description, count, Icon }) => (
          <Item key={view} asChild>
            <button type="button" onClick={() => onChangeView(view)}>
              <ItemMedia variant="icon" aria-hidden="true">
                <Icon size={18} stroke={1.6} />
              </ItemMedia>
              <ItemContent>
                <ItemTitle>{title}</ItemTitle>
                <ItemDescription>{description}</ItemDescription>
              </ItemContent>
              <ItemActions>
                <Badge>{count}</Badge>
                <IconChevronRight size={15} stroke={1.6} aria-hidden="true" />
              </ItemActions>
            </button>
          </Item>
        ))}
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
            <p>
              {artifactCoverage
                ? "No artifact metadata in the loaded messages."
                : "No artifacts have been released."}
            </p>
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
        {artifactCoverage ? (
          <p className="field-help">{artifactCoverage}</p>
        ) : null}
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
        {error !== "" && sessionMap !== null ? (
          <p className="session-map-stale-note" role="status">
            Could not refresh durable records. Showing the last loaded records.{" "}
            {error}
          </p>
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
