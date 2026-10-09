import { useEffect, useState } from "react";
import {
  ArtifactLibrary,
  Button,
  WorkspaceSurfaceHeader,
  type LibraryArtifact,
} from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconPlugConnected } from "@tabler/icons-react";
import {
  projectPath,
  request,
  type FleetNode,
  type Update,
  type ThreadDetailResponse,
} from "./api";
import { useResource, errorMessage } from "./resources";
import { LoadState } from "./Home";
import { RouteLink } from "./navigation";
import { hostHref } from "./routes";
import { workspaceName } from "./workspace-navigation";
import "@colossus/ui/styles/library.css";

export function WorkspaceCapabilities({ agent }: { agent: FleetNode }) {
  const posture = agent.node.policy;
  return (
    <section className="workspace-resource-page">
      <WorkspaceSurfaceHeader
        eyebrow="Capabilities / This Workspace"
        title="Derived capability catalog"
        description="A read-only view derived from the selected runtime, access profile, tools, workflows, and agents."
      />
      <div className="workspace-resource-body">
        {!posture ? (
          <p className="sync-notice">
            The runtime has not reported its capability catalog.
          </p>
        ) : (
          <>
            <dl className="configuration-list">
              <div>
                <dt>Access profile</dt>
                <dd>{posture.access_profile.replaceAll("_", " ")}</dd>
              </div>
              <div>
                <dt>Execution boundary</dt>
                <dd>
                  {posture.sandbox_backend} · {posture.sandbox_profile}
                </dd>
              </div>
            </dl>
            {[
              ["Roles", posture.allowed_roles],
              ["Tools", posture.allowed_tools],
              ["Available orchestration", posture.capabilities],
            ].map(([name, values]) => (
              <section key={String(name)} className="configuration-group">
                <h3>{name}</h3>
                <div className="configuration-tags">
                  {(values as string[]).map((value) => (
                    <span key={value}>{value}</span>
                  ))}
                </div>
              </section>
            ))}
          </>
        )}
      </div>
    </section>
  );
}
export function WorkspaceConnections({
  project,
  agent,
  busy,
  onRevoke,
  canRevoke,
}: {
  project: string;
  agent: FleetNode;
  busy: boolean;
  onRevoke?: (() => void) | undefined;
  canRevoke: boolean;
}) {
  return (
    <section className="workspace-resource-page">
      <WorkspaceSurfaceHeader
        eyebrow="Connections / Runtime routing"
        title="Connections"
        description="Workspaces own their runtime connection to the Control Plane."
      />
      <div className="workspace-resource-body">
        <section className="control-card">
          <header>
            <h3>
              <IconPlugConnected size={18} aria-hidden="true" />{" "}
              {workspaceName(agent)}
            </h3>
          </header>
          <dl className="configuration-list">
            {[
              [
                "Connection",
                agent.node.revoked
                  ? "Revoked"
                  : agent.presence?.ready
                    ? "Connected"
                    : "Offline",
              ],
              ["Runtime identity", agent.node.instance_id],
              ["Workspace identity", agent.node.workspace_id ?? "Not reported"],
              ["Enrollment alias", agent.node.label],
              ["Allowed roles", agent.node.roles.join(", ")],
            ].map(([name, value]) => (
              <div key={name}>
                <dt>{name}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
          {agent.node.host_id ? (
            <RouteLink href={hostHref(project, agent.node.host_id)}>
              Open host
            </RouteLink>
          ) : null}
          {canRevoke && onRevoke ? (
            <Button
              variant="danger"
              disabled={busy || agent.node.revoked}
              onClick={onRevoke}
            >
              Revoke workspace connection
            </Button>
          ) : null}
        </section>
      </div>
    </section>
  );
}
interface Plugin {
  digest: string;
  available: boolean;
  status: string;
  manifest: {
    name: string;
    description?: string | null;
    version?: string | null;
  };
  skills: { id: string; name?: string }[];
}
export function WorkspacePlugins({
  project,
  agent,
}: {
  project: string;
  agent: FleetNode;
}) {
  const [state, setState] = useState<{
    items: Plugin[];
    loading: boolean;
    error: string;
  }>({ items: [], loading: true, error: "" });
  const [generation, setGeneration] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    setState({ items: [], loading: true, error: "" });
    void (async () => {
      try {
        if (!agent.presence?.ready || agent.node.revoked)
          throw new Error(
            "Connect this Workspace to inspect its Agent Plugins.",
          );
        const path = `${projectPath(project)}/nodes/${encodeURIComponent(agent.node.node_id)}/resources`;
        const context = await request<{
          kind: string;
          connection_id: string;
          value?: { capabilities: string[] };
          error?: { message: string };
        }>(
          path,
          { connection_id: null, operation: { operation: "context" } },
          abort.signal,
        );
        if (context.kind !== "result")
          throw new Error(
            context.error?.message ?? "Plugin discovery is unavailable.",
          );
        if (!context.value?.capabilities.includes("plugins.read"))
          throw new Error(
            "The dedicated cloud application has no plugin discovery grant. Manage plugin installation and local access in Desktop.",
          );
        const result = await request<{
          kind: string;
          value?: Plugin[];
          error?: { message: string };
        }>(
          path,
          {
            connection_id: context.connection_id,
            operation: { operation: "list_plugins" },
          },
          abort.signal,
        );
        if (result.kind !== "result" || !result.value)
          throw new Error(
            result.error?.message ?? "Plugin discovery is unavailable.",
          );
        if (!abort.signal.aborted)
          setState({ items: result.value, loading: false, error: "" });
      } catch (error) {
        if (!abort.signal.aborted)
          setState({ items: [], loading: false, error: errorMessage(error) });
      }
    })();
    return () => abort.abort();
  }, [
    project,
    agent.node.node_id,
    agent.node.revoked,
    agent.presence?.ready,
    generation,
  ]);
  const columns: DataTableColumn<Plugin>[] = [
    {
      id: "name",
      label: "Agent Plugin",
      value: (item) => item.manifest.name,
      cell: (item) => (
        <span>
          <strong>{item.manifest.name}</strong>
          <small>{item.manifest.description}</small>
        </span>
      ),
    },
    {
      id: "status",
      label: "Status",
      value: (item) => (item.available ? "Available" : item.status),
    },
    { id: "skills", label: "Skills", value: (item) => item.skills.length },
    {
      id: "version",
      label: "Version",
      value: (item) => item.manifest.version ?? "Not reported",
    },
  ];
  return (
    <section className="workspace-resource-page">
      <WorkspaceSurfaceHeader
        eyebrow="Plugins / This Workspace"
        title="Agent Plugins"
        description="Caller-authorized discovery from the selected runtime. Installation and credentials remain managed in Desktop."
        actions={
          <Button
            disabled={state.loading}
            onClick={() => setGeneration((value) => value + 1)}
          >
            Refresh
          </Button>
        }
      />
      <div className="workspace-resource-body">
        <LoadState
          {...state}
          retry={() => setGeneration((value) => value + 1)}
        />
        {!state.loading && !state.error ? (
          <DataTable
            data={state.items}
            columns={columns}
            getRowId={(item) => item.digest}
            label="Agent Plugins"
            itemLabel="plugins"
            search={{ columnId: "name", label: "Search Agent Plugins" }}
          />
        ) : null}
      </div>
    </section>
  );
}
export function releasedArtifacts(updates: Update[]): LibraryArtifact[] {
  const items = new Map<string, LibraryArtifact>();
  for (const update of updates) {
    const message = update.update.message as
      | {
          content?: {
            artifact?: {
              artifact_id?: string;
              file_name?: string;
              media_type?: string;
              byte_length?: number;
              purpose?: string;
              state?: string;
            };
          }[];
        }
      | undefined;
    for (const part of message?.content ?? []) {
      const artifact = part.artifact;
      if (!artifact?.artifact_id || !artifact.file_name) continue;
      items.set(artifact.artifact_id, {
        key: artifact.artifact_id,
        fileName: artifact.file_name,
        typeLabel: artifact.media_type ?? "File",
        sizeLabel:
          typeof artifact.byte_length === "number"
            ? `${artifact.byte_length.toLocaleString()} bytes`
            : "Size not reported",
        purposeLabel: artifact.purpose ?? "Released output",
        stateLabel: artifact.state ?? "Released metadata",
        canOpen: false,
      });
    }
  }
  return [...items.values()];
}
export function WorkspaceLibrary({
  project,
  thread,
}: {
  project: string;
  thread: string | null;
}) {
  const detail = useResource<ThreadDetailResponse>(
    thread
      ? `${projectPath(project)}/threads/${encodeURIComponent(thread)}`
      : null,
  );
  const [state, setState] = useState<{
    scope: string;
    items: LibraryArtifact[];
    loading: boolean;
    error: string;
    bounded: boolean;
  }>({ scope: "", items: [], loading: false, error: "", bounded: false });
  const scope = project + ":" + (thread ?? "");
  useEffect(() => {
    const abort = new AbortController();
    const tasks = detail.data?.tasks ?? [];
    if (!thread || !detail.data) {
      setState({ scope, items: [], loading: false, error: "", bounded: false });
      return;
    }
    setState({ scope, items: [], loading: true, error: "", bounded: false });
    void (async () => {
      try {
        const updates: Update[] = [];
        let bounded =
          Boolean(detail.data?.next_task_cursor) ||
          detail.data?.thread.sync_status !== "current";
        for (const task of tasks.slice(0, 8)) {
          const page = await request<{
            updates: Update[];
            next_after: number | null;
          }>(
            `${projectPath(project)}/tasks/${encodeURIComponent(task.task_id)}/updates`,
            undefined,
            abort.signal,
          );
          updates.push(...page.updates);
          bounded ||= page.next_after !== null;
        }
        bounded ||= tasks.length > 8;
        if (!abort.signal.aborted)
          setState({
            scope,
            items: releasedArtifacts(updates),
            loading: false,
            error: "",
            bounded,
          });
      } catch (error) {
        if (!abort.signal.aborted)
          setState({
            scope,
            items: [],
            loading: false,
            error: errorMessage(error),
            bounded: false,
          });
      }
    })();
    return () => abort.abort();
  }, [scope, thread, detail.data, project]);
  const current =
    state.scope === scope
      ? state
      : { items: [], loading: true, error: "", bounded: false };
  return (
    <section className="workspace-resource-page">
      <ArtifactLibrary
        artifacts={current.items}
        coverage={
          <>
            <LoadState
              loading={detail.loading || current.loading}
              error={detail.error || current.error}
              retry={detail.refresh}
            />
            <p className="field-help">
              {!thread
                ? "Select a conversation on the left to inspect its released artifacts."
                : current.bounded
                  ? "This inventory covers the loaded retained run messages. Additional local artifacts and earlier history may be available in Desktop."
                  : "Released artifact metadata from the selected conversation’s retained run messages."}
            </p>
          </>
        }
      />
    </section>
  );
}

export function WorkspaceResourceView({
  view,
  project,
  agent,
  contextThread,
  busy,
  onRevoke,
  canRevoke,
}: {
  view: "capabilities" | "plugins" | "library" | "connections";
  project: string;
  agent: FleetNode;
  contextThread: string | null;
  busy: boolean;
  onRevoke?: (() => void) | undefined;
  canRevoke: boolean;
}) {
  if (view === "capabilities") return <WorkspaceCapabilities agent={agent} />;
  if (view === "plugins")
    return <WorkspacePlugins project={project} agent={agent} />;
  if (view === "library")
    return <WorkspaceLibrary project={project} thread={contextThread} />;
  return (
    <WorkspaceConnections
      project={project}
      agent={agent}
      busy={busy}
      onRevoke={onRevoke}
      canRevoke={canRevoke}
    />
  );
}
