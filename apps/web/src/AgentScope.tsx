import { useEffect, useRef, useState } from "react";
import {
  Button,
  DropdownSelect,
  WorkspaceSidebarHeading,
  WorkspaceSidebarSearch,
  WorkspaceSidebarScope,
  WorkspaceSidebarWorkspace,
  WorkspaceSidebarGroupHeading,
  WorkspaceSidebarThreadContent,
  WorkWelcome,
} from "@colossus/ui";
import {
  IconArrowLeft,
  IconFolder,
  IconPlus,
  IconShield,
  IconChevronDown,
  IconPencilPlus,
  IconCheck,
  IconLock,
  IconAlertTriangle,
} from "@tabler/icons-react";
import {
  projectPath,
  request,
  taskStatus,
  statusLabel,
  type Task,
  type FleetNode,
  type Host,
  type Permission,
  type Thread,
} from "./api";
import type { AgentPolicy } from "./control-api";
import { useResource } from "./resources";
import { workspaceName, workspaceState } from "./workspace-navigation";
import { AnalyticsPanel, LoadState, Metrics } from "./Home";
import { SectionTabs } from "./Projects";
import { RunComposer, type RunRequest } from "./RunComposer";
import { RouteLink } from "./navigation";
import { agentHref, globalHref, threadHref, type AgentView } from "./routes";
export function AgentSidebar({
  project,
  nodes,
  hosts,
  agentId,
  selected,
  onAgent,
  onOpen,
  onNew,
  onBack,
  tasks = [],
  newDisabled = false,
}: {
  project: string;
  nodes: FleetNode[];
  hosts: Host[];
  agentId: string;
  selected: string;
  onAgent: (id: string) => void;
  onOpen: (thread: Thread) => void;
  onNew: () => void;
  onBack: () => void;
  tasks?: Task[];
  newDisabled?: boolean;
}) {
  const [query, setQuery] = useState(""),
    [search, setSearch] = useState(""),
    [archive, setArchive] = useState("false"),
    [scope, setScope] = useState<"all" | "workspace">("workspace"),
    [expanded, setExpanded] = useState(true),
    [choosing, setChoosing] = useState(false),
    [older, setOlder] = useState<Thread[]>([]),
    [loadedScope, setLoadedScope] = useState(""),
    [nextCursor, setNextCursor] = useState<string | null>(null),
    [moreLoaded, setMoreLoaded] = useState(false),
    [moreBusy, setMoreBusy] = useState(false),
    [moreError, setMoreError] = useState("");
  const searchRef = useRef<HTMLInputElement>(null),
    alive = useRef(true),
    currentScope = useRef("");
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), 250);
    return () => clearTimeout(timer);
  }, [query]);
  const scopeKey = JSON.stringify([
    project,
    scope === "workspace" ? agentId : null,
    search,
    archive,
  ]);
  currentScope.current = scopeKey;
  useEffect(() => {
    setOlder([]);
    setLoadedScope("");
    setNextCursor(null);
    setMoreLoaded(false);
    setMoreError("");
    setMoreBusy(false);
  }, [scopeKey]);
  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if (
        (event.metaKey || event.ctrlKey) &&
        event.key.toLowerCase() === "k" &&
        !event.isComposing
      ) {
        event.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", focus);
    return () => window.removeEventListener("keydown", focus);
  }, []);
  const params = new URLSearchParams({ archived: archive, limit: "25" });
  if (scope === "workspace") params.set("node_id", agentId);
  if (search) params.set("query", search);
  const path = `${projectPath(project)}/threads?${params}`;
  const resource = useResource<{
    threads: Thread[];
    next_cursor?: string | null;
  }>(path, 3000);
  const visible = new Map<string, Thread>();
  for (const thread of loadedScope === scopeKey ? older : [])
    visible.set(thread.thread_id, thread);
  for (const thread of resource.data?.threads ?? [])
    visible.set(thread.thread_id, thread);
  const threads = (resource.error ? [] : [...visible.values()])
    .filter(
      (thread) =>
        thread.project_id === project &&
        (scope === "all" || thread.node_id === agentId) &&
        thread.archived === (archive === "true"),
    )
    .sort(
      (a, b) =>
        b.updated_at.localeCompare(a.updated_at) ||
        a.thread_id.localeCompare(b.thread_id),
    );
  const agent = nodes.find((item) => item.node.node_id === agentId),
    host = hosts.find((item) => item.host_id === agent?.node.host_id),
    cursor =
      loadedScope === scopeKey && moreLoaded
        ? nextCursor
        : resource.data?.next_cursor;
  async function loadOlder() {
    if (!cursor || moreBusy) return;
    const expected = scopeKey;
    setMoreBusy(true);
    setMoreError("");
    try {
      const value = await request<{
        threads: Thread[];
        next_cursor?: string | null;
      }>(`${path}&after=${encodeURIComponent(cursor)}`);
      if (!alive.current || currentScope.current !== expected) return;
      setOlder((current) => {
        const map = new Map(current.map((item) => [item.thread_id, item]));
        for (const item of [
          ...(resource.data?.threads ?? []),
          ...value.threads,
        ])
          map.set(item.thread_id, item);
        return [...map.values()];
      });
      setLoadedScope(expected);
      setNextCursor(value.next_cursor ?? null);
      setMoreLoaded(true);
    } catch (error) {
      if (alive.current && currentScope.current === expected)
        setMoreError(
          error instanceof Error
            ? error.message
            : "Earlier threads could not be loaded.",
        );
    } finally {
      if (alive.current && currentScope.current === expected)
        setMoreBusy(false);
    }
  }
  const ready = agent?.presence?.ready && !agent.node.revoked,
    state = agent?.node.revoked
      ? "Revoked"
      : ready
        ? "Ready"
        : agent?.presence
          ? "Starting"
          : "Offline";
  return (
    <div className="agent-sidebar">
      <RouteLink
        className="ui-button ui-button--tertiary scope-fleet-back"
        href={globalHref("fleet", project)}
        onNavigate={onBack}
        back
      >
        <IconArrowLeft size={15} aria-hidden="true" />
        Back
      </RouteLink>
      <WorkspaceSidebarHeading />
      <WorkspaceSidebarSearch
        value={query}
        onChange={setQuery}
        inputRef={searchRef}
        maxLength={256}
        shortcut={
          navigator.platform.toLowerCase().includes("mac") ? "⌘K" : "Ctrl K"
        }
      />
      <WorkspaceSidebarScope
        value={scope}
        onChange={setScope}
        workspaceDisabled={!agent}
      />
      <div className="scope-filter-line">
        <span>
          {scope === "all"
            ? "In this project"
            : (host?.label ?? "Assigned workspace")}
        </span>
        <DropdownSelect
          aria-label="Conversation archive filter"
          value={archive}
          onChange={(event) => setArchive(event.target.value)}
        >
          <option value="false">Open</option>
          <option value="true">Archived</option>
        </DropdownSelect>
      </div>
      <WorkspaceSidebarWorkspace
        identity={
          <RouteLink
            className="shared-workspace-row-identity"
            href={agentHref(project, agentId)}
            onNavigate={() => onAgent(agentId)}
            title={agent?.node.label ?? agentId}
          >
            <IconFolder size={17} stroke={1.7} aria-hidden="true" />
            <strong>
              {agent?.node.workspace_label ?? agent?.node.label ?? "Workspace"}
            </strong>
          </RouteLink>
        }
        state={
          <span className="shared-workspace-readiness">
            <span className={`dot ${ready ? "live" : ""}`} />
            {state}
          </span>
        }
        actions={
          <>
            <button
              className="scope-workspace-toggle"
              type="button"
              aria-label="Choose workspace agent"
              aria-expanded={choosing}
              onClick={() => setChoosing((value) => !value)}
            >
              <IconChevronDown size={16} aria-hidden="true" />
            </button>
            <button
              className="scope-new-thread"
              type="button"
              aria-label="New conversation"
              title="New conversation"
              disabled={newDisabled || !agent || agent.node.revoked}
              onClick={onNew}
            >
              <IconPencilPlus size={17} stroke={1.8} aria-hidden="true" />
            </button>
          </>
        }
      />
      {choosing ? (
        <div className="scope-workspace-choices">
          <DropdownSelect
            aria-label="Select workspace agent"
            value={agentId}
            onChange={(event) => {
              setChoosing(false);
              onAgent(event.target.value);
            }}
          >
            {nodes.map((item) => (
              <option key={item.node.node_id} value={item.node.node_id}>
                {item.node.workspace_label
                  ? `${item.node.workspace_label} · `
                  : ""}
                {item.node.label}
              </option>
            ))}
          </DropdownSelect>
          <p>
            Showing {nodes.length} loaded agents in this project. Fleet provides
            the full paged inventory.
          </p>
        </div>
      ) : null}
      <div className="scope-recent-heading">
        <WorkspaceSidebarGroupHeading
          label={search ? "Results" : "Recent"}
          count={threads.length}
        />
        <button
          className="scope-recent-toggle"
          type="button"
          aria-label={
            expanded ? "Collapse recent threads" : "Expand recent threads"
          }
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          <IconChevronDown size={14} aria-hidden="true" />
        </button>
      </div>
      <div className="scope-thread-stack" hidden={!expanded}>
        <LoadState {...resource} retry={resource.refresh} />
        <nav
          className="scope-compact-thread-list"
          aria-label="Agent conversations"
        >
          {threads.map((thread) => {
            const task = thread.active_task_id
              ? tasks.find(
                  (item) =>
                    item.task_id === thread.active_task_id &&
                    item.project_id === project &&
                    item.node_id === thread.node_id,
                )
              : tasks
                  .filter(
                    (item) =>
                      item.thread_id === thread.thread_id &&
                      item.project_id === project &&
                      item.node_id === thread.node_id,
                  )
                  .sort((a, b) =>
                    (
                      b.created_at ??
                      b.snapshot?.run.created_at ??
                      ""
                    ).localeCompare(
                      a.created_at ?? a.snapshot?.run.created_at ?? "",
                    ),
                  )[0];
            const state = task
              ? taskStatus(task)
              : thread.active_task_id
                ? "active"
                : thread.can_continue
                  ? ""
                  : "read_only";
            const date = new Date(thread.updated_at),
              updated = Number.isFinite(date.getTime())
                ? date.toLocaleString([], {
                    month: "short",
                    day: "numeric",
                    hour: "numeric",
                    minute: "2-digit",
                  })
                : "Date unavailable";
            const workspace = nodes.find(
              (item) => item.node.node_id === thread.node_id,
            )?.node;
            const metadata = [
              scope === "all"
                ? (workspace?.workspace_label ??
                  workspace?.label ??
                  thread.node_id)
                : null,
              task ? statusLabel(task.request.mode) : null,
              updated,
              thread.archived ? "Archived" : null,
              !thread.can_continue ? "View only" : null,
              thread.sync_status === "incomplete" ? "Incomplete" : null,
            ]
              .filter(Boolean)
              .join(" · ");
            const status =
              state === "completed" ? (
                <IconCheck size={13} stroke={2.2} aria-hidden="true" />
              ) : state === "read_only" ? (
                <IconLock size={13} aria-hidden="true" />
              ) : ["failed", "outcome_unknown"].includes(state) ? (
                <IconAlertTriangle size={13} aria-hidden="true" />
              ) : state ? (
                <span className="dot" />
              ) : null;
            return (
              <RouteLink
                className="shared-workspace-thread-link"
                key={thread.thread_id}
                href={threadHref(project, thread.thread_id)}
                onNavigate={() => onOpen(thread)}
                aria-current={
                  selected === thread.thread_id ? "page" : undefined
                }
              >
                <WorkspaceSidebarThreadContent
                  title={thread.title || "Untitled conversation"}
                  metadata={metadata}
                  status={
                    status ? (
                      <span
                        className={`scope-thread-status ${state === "completed" ? "is-success" : ["failed", "outcome_unknown"].includes(state) ? "is-danger" : ""}`}
                        title={statusLabel(state)}
                      >
                        <span className="sr-only">{statusLabel(state)}</span>
                        {status}
                      </span>
                    ) : null
                  }
                />
              </RouteLink>
            );
          })}
        </nav>
        {!resource.loading && !resource.error && !threads.length ? (
          <p className="empty-copy">
            {search
              ? "No matching threads."
              : "No shared conversations in this scope."}
          </p>
        ) : null}
        {cursor ? (
          <Button
            variant="tertiary"
            className="scope-load-older"
            disabled={resource.loading || moreBusy}
            onClick={() => void loadOlder()}
          >
            {moreBusy ? "Loading history…" : "Load older threads"}
          </Button>
        ) : null}
        {moreError ? (
          <p className="sidebar-error" role="alert">
            {moreError}
          </p>
        ) : null}
        {threads.length ? (
          <p className="scope-history-note">
            {threads.length} loaded{" "}
            {threads.length === 1 ? "thread" : "threads"}
            {cursor ? " · more history available" : ""}
          </p>
        ) : null}
      </div>
    </div>
  );
}
export function AgentWorkspace({
  project,
  agent,
  nodes,
  permissions,
  creating,
  busy,
  onCreate,
  onNew,
  initialTab = "overview",
  view,
  onView,
  onOpen,
  onRevoke,
}: {
  project: string;
  agent: FleetNode | undefined;
  nodes: FleetNode[];
  permissions: Permission[];
  creating: boolean;
  busy: boolean;
  onCreate: (request: RunRequest) => Promise<boolean>;
  onNew: () => void;
  initialTab?: string;
  view?: AgentView;
  onView?: (value: AgentView) => void;
  onOpen?: (thread: Thread) => void;
  onRevoke?: () => void;
}) {
  const [localTab, setTab] = useState(initialTab);
  const [draftSeed, setDraftSeed] = useState<
    { key: string; text: string } | undefined
  >();
  const tab = view ?? localTab;
  const changeTab = (value: string) =>
    onView ? onView(value as AgentView) : setTab(value);
  if (creating)
    return (
      <section className="agent-new-work" aria-labelledby="new-thread-heading">
        <header className="page-heading">
          <div>
            <span className="eyebrow">
              Fleet /{" "}
              {agent?.node.workspace_label ?? agent?.node.label ?? "Agent"}
            </span>
            <h1 id="new-thread-heading">New thread</h1>
          </div>
          <span className="shared-workspace-readiness">
            <span className={`dot ${agent?.presence?.ready ? "live" : ""}`} />
            {agent?.node.revoked
              ? "Enrollment revoked"
              : agent?.presence?.ready
                ? "Agent ready"
                : agent?.presence
                  ? "Agent starting"
                  : "Agent offline"}
          </span>
        </header>
        <div className="agent-new-work-welcome">
          <WorkWelcome
            eyebrow="Connected agent workspace"
            description="Send work to this agent, switch to plan mode, or continue a conversation. Its runtime retains tools, approvals, and canonical context."
            disabled={
              busy ||
              !permissions.includes("execute") ||
              !agent ||
              agent.node.revoked
            }
            onSuggestion={(text) =>
              setDraftSeed({ key: crypto.randomUUID(), text })
            }
          />
        </div>
        <div className="agent-new-work-composer">
          <RunComposer
            draftSeed={draftSeed}
            onPolicy={() => changeTab("policy")}
            nodes={nodes}
            nodeId={agent?.node.node_id ?? ""}
            lockedRuntime
            busy={busy}
            disabled={
              !permissions.includes("execute") || !agent || agent.node.revoked
            }
            label="Start a conversation"
            action="Start conversation"
            onSubmit={onCreate}
          />
        </div>
      </section>
    );
  return (
    <section className="control-page agent-workspace">
      <header className="page-heading">
        <div>
          <span className="eyebrow">Host / workspace</span>
          <h1>{agent ? workspaceName(agent) : "Workspace"}</h1>
          <p>
            {agent
              ? workspaceState(agent)
              : "Open a conversation from the sidebar or start new work."}
          </p>
        </div>
        <Button
          variant="primary"
          disabled={
            !permissions.includes("execute") || !agent || agent.node.revoked
          }
          onClick={onNew}
        >
          <IconPlus size={16} aria-hidden="true" />
          New conversation
        </Button>
      </header>
      <SectionTabs
        current={tab}
        onChange={changeTab}
        hrefFor={
          agent
            ? (value) =>
                agentHref(project, agent.node.node_id, value as AgentView)
            : undefined
        }
        items={[
          { id: "overview", label: "Overview" },
          { id: "threads", label: "Conversations" },
          { id: "analytics", label: "Analytics" },
          { id: "policy", label: "Policy & configuration" },
        ]}
      />
      {agent && tab === "overview" ? (
        <details className="workspace-connection-details">
          <summary>Connection details</summary>
          <dl>
            <dt>Enrollment alias</dt>
            <dd>{agent.node.label}</dd>
            <dt>Runtime identity</dt>
            <dd className="mono">{agent.node.instance_id}</dd>
            <dt>Agent identity</dt>
            <dd className="mono">{agent.node.node_id}</dd>
            <dt>Allowed roles</dt>
            <dd>{agent.node.roles.join(", ")}</dd>
          </dl>
          {permissions.includes("administer") && onRevoke ? (
            <Button
              variant="danger"
              disabled={busy || agent.node.revoked}
              onClick={onRevoke}
            >
              Revoke workspace connection
            </Button>
          ) : null}
        </details>
      ) : null}
      {agent ? (
        tab === "threads" ? (
          <AgentConversations
            project={project}
            node={agent.node.node_id}
            onOpen={onOpen}
          />
        ) : tab === "overview" || tab === "analytics" ? (
          <AnalyticsPanel
            path={`${projectPath(project)}/nodes/${encodeURIComponent(agent.node.node_id)}/analytics`}
          />
        ) : (
          <AgentPolicyPanel
            path={`${projectPath(project)}/nodes/${encodeURIComponent(agent.node.node_id)}/policy`}
          />
        )
      ) : (
        <p className="empty-copy">Agent inventory is loading.</p>
      )}
    </section>
  );
}
function AgentConversations({
  project,
  node,
  onOpen,
}: {
  project: string;
  node: string;
  onOpen: ((thread: Thread) => void) | undefined;
}) {
  const [cursors, setCursors] = useState([""]);
  const params = new URLSearchParams({
    node_id: node,
    archived: "false",
    limit: "25",
  });
  if (cursors.at(-1)) params.set("after", cursors.at(-1)!);
  const resource = useResource<{
    threads: Thread[];
    next_cursor?: string | null;
  }>(`${projectPath(project)}/threads?${params}`, 3000);
  return (
    <section className="control-card">
      <header>
        <h3>Agent conversations</h3>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {resource.data?.threads.length ? (
        <ul className="recent-list">
          {resource.data.threads.map((thread) => (
            <li key={thread.thread_id}>
              <RouteLink
                href={threadHref(project, thread.thread_id)}
                onNavigate={() => onOpen?.(thread)}
              >
                <span>
                  <strong>{thread.title || "Untitled conversation"}</strong>
                  <small>
                    {thread.can_continue ? "Conversation" : "Read-only history"}
                    {thread.sync_status === "incomplete" ? " · Incomplete" : ""}
                  </small>
                </span>
                <time dateTime={thread.updated_at}>
                  {new Date(thread.updated_at).toLocaleDateString()}
                </time>
              </RouteLink>
            </li>
          ))}
        </ul>
      ) : !resource.loading && !resource.error ? (
        <p className="empty-copy">No shared conversations in this workspace.</p>
      ) : null}
      <div className="scope-pagination">
        <Button
          variant="tertiary"
          disabled={resource.loading || cursors.length === 1}
          onClick={() => setCursors((value) => value.slice(0, -1))}
        >
          Previous
        </Button>
        <span>{cursors.length}</span>
        <Button
          variant="tertiary"
          disabled={resource.loading || !resource.data?.next_cursor}
          onClick={() => {
            if (resource.data?.next_cursor)
              setCursors((value) => [...value, resource.data!.next_cursor!]);
          }}
        >
          Next
        </Button>
      </div>
    </section>
  );
}
export function AgentPolicyPanel({ path }: { path: string }) {
  const resource = useResource<AgentPolicy>(path, 15000),
    data = resource.data;
  return (
    <>
      <LoadState {...resource} retry={resource.refresh} />
      {data ? (
        <>
          <section className="control-card">
            <header>
              <h3>
                <IconShield size={17} aria-hidden="true" />
                Runtime policy posture
              </h3>
              <span
                className={`status ${data.evaluation.status === "aligned" ? "status-completed" : data.evaluation.status === "drift" ? "status-waiting" : "status-cancelled"}`}
              >
                {data.evaluation.status === "aligned"
                  ? "Aligned with baseline"
                  : data.evaluation.status === "drift"
                    ? "Configuration drift"
                    : "Evaluation unavailable"}
              </span>
            </header>
            <p className="field-help">{data.description}</p>
            <dl className="stat-list">
              <div>
                <dt>Source</dt>
                <dd>Authenticated runtime report</dd>
              </div>
              <div>
                <dt>Observed</dt>
                <dd>
                  {data.observed_at
                    ? new Date(data.observed_at * 1000).toLocaleString()
                    : "Not reported"}
                  {data.stale ? " · Stale" : ""}
                </dd>
              </div>
              <div>
                <dt>Connection</dt>
                <dd>{data.connected ? "Connected" : "Offline"}</dd>
              </div>
            </dl>
            {data.evaluation.findings.length ? (
              <ul className="policy-findings">
                {data.evaluation.findings.map((finding) => (
                  <li key={finding.code}>{finding.description}</li>
                ))}
              </ul>
            ) : null}
            {data.stale ? (
              <p className="sync-notice">
                The latest report is stale. Drift evaluation resumes when the
                agent reports current posture.
              </p>
            ) : null}
          </section>
          {data.posture ? (
            <section className="control-card">
              <header>
                <h3>Reported configuration</h3>
              </header>
              <dl className="configuration-list">
                {[
                  ["Access profile", data.posture.access_profile],
                  [
                    "Sandbox",
                    `${data.posture.sandbox_backend} · ${data.posture.sandbox_profile}`,
                  ],
                  [
                    "Approval mode",
                    data.posture.approval_mode.replaceAll("_", " "),
                  ],
                  [
                    "Model routing",
                    data.posture.models
                      .map((model) => `${model.profile}: ${model.label}`)
                      .join(" · ") || "Not reported",
                  ],
                  [
                    "Configuration revision",
                    data.posture.configuration_revision ?? "Not reported",
                  ],
                  ["Configuration fingerprint", data.posture.fingerprint],
                ].map(([label, value]) => (
                  <div key={label}>
                    <dt>{label}</dt>
                    <dd>{value}</dd>
                  </div>
                ))}
              </dl>
              {[
                ["Roles", data.posture.allowed_roles],
                ["Allowed tools", data.posture.allowed_tools],
                ["Capabilities", data.posture.capabilities],
              ].map(([label, values]) => (
                <div className="configuration-group" key={label as string}>
                  <h4>{label as string}</h4>
                  <div className="configuration-tags">
                    {(values as string[]).length ? (
                      (values as string[]).map((value) => (
                        <span key={value}>{value}</span>
                      ))
                    ) : (
                      <span>None</span>
                    )}
                  </div>
                </div>
              ))}
              {data.posture.findings.length ? (
                <div className="configuration-group">
                  <h4>Reported configuration findings</h4>
                  <ul className="policy-findings">
                    {data.posture.findings.map((finding) => (
                      <li key={finding.code}>{findingLabel(finding.code)}</li>
                    ))}
                  </ul>
                </div>
              ) : null}
              <div className="configuration-group">
                <h4>Canonical policy counters</h4>
                <Metrics
                  items={[
                    {
                      label: "Denied requests",
                      value:
                        data.posture.telemetry.denied_requests ?? "Unavailable",
                    },
                    {
                      label: "Approval requests",
                      value:
                        data.posture.telemetry.approval_requests ??
                        "Unavailable",
                    },
                    {
                      label: "Unknown outcomes",
                      value:
                        data.posture.telemetry.outcome_unknown_runs ??
                        "Unavailable",
                    },
                  ]}
                />
                <p className="field-help">
                  These counters require a canonical runtime report. Run
                  failures and reported configuration findings are not
                  enforcement violation counts.
                </p>
              </div>
            </section>
          ) : (
            <p className="empty-copy">
              This runtime has not released its policy configuration.
            </p>
          )}
        </>
      ) : null}
    </>
  );
}

function findingLabel(code: string) {
  return (
    (
      {
        "storage.ephemeral": "Runtime journal storage is ephemeral.",
        "storage.plaintext": "Runtime journal storage is plaintext.",
        "sandbox.danger_full_access":
          "Runtime is configured for acknowledged full host access.",
        "observability.sensitive_journal_payloads":
          "Runtime retains sensitive journal payloads.",
        "credentials.mcp_oauth_plaintext":
          "Runtime stores MCP OAuth credentials as plaintext.",
      } as Record<string, string>
    )[code] ?? code
  );
}
