import { useEffect, useRef, useState } from "react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@colossus/ui/components/ui/collapsible";
import { Button as FoundationButton } from "@colossus/ui/components/ui/button";
import {
  SidebarHeader,
  SidebarContent,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarMenuSubButton,
} from "@colossus/ui/components/ui/sidebar";
import {
  Button,
  WorkspaceNavigation,
  DropdownSelect,
  WorkspaceSidebarHeading,
  WorkspaceSidebarSearch,
  WorkspaceSidebarWorkspace,
  WorkspaceSidebarThreadContent,
} from "@colossus/ui";
import {
  IconArrowLeft,
  IconChevronDown,
  IconDeviceDesktop,
  IconFolder,
  IconPencilPlus,
  IconLock,
} from "@tabler/icons-react";
import {
  projectPath,
  request,
  type FleetNode,
  type Host,
  type Thread,
} from "./api";
import { useResource } from "./resources";
import { RouteLink } from "./navigation";
import { agentHref, globalHref, threadHref } from "./routes";
import { LoadState } from "./Home";
import {
  WORKSPACE_VIEW_ROUTES,
  hostConnection,
  platformName,
  workspaceName,
  workspaceState,
} from "./workspace-navigation";

/** Host-specific paging never depends on which project inventory page was loaded. */
export function useHostRoster(
  project: string,
  hostId: string,
  identity: string,
) {
  const path = hostId
    ? `${projectPath(project)}/nodes?host_id=${encodeURIComponent(hostId)}&limit=32`
    : null;
  const scope = JSON.stringify([identity, project, hostId]);
  const resource = useResource<{
    nodes: FleetNode[];
    next_cursor?: string | null;
  }>(path, 5000, identity);
  const [history, setHistory] = useState<{
    scope: string;
    nodes: FleetNode[];
    cursor: string | null;
  }>({ scope, nodes: [], cursor: null });
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const current = useRef(scope),
    alive = useRef(true);
  current.current = scope;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    setHistory({ scope, nodes: [], cursor: null });
    setBusy(false);
    setError("");
  }, [scope]);
  const map = new Map<string, FleetNode>();
  if (history.scope === scope)
    for (const item of history.nodes) map.set(item.node.node_id, item);
  for (const item of resource.data?.nodes ?? [])
    map.set(item.node.node_id, item);
  const nodes = resource.error
    ? []
    : [...map.values()].filter(
        (item) =>
          item.node.project_id === project && item.node.host_id === hostId,
      );
  const cursor =
    history.scope === scope && history.nodes.length
      ? history.cursor
      : resource.data?.next_cursor;
  async function more() {
    if (!path || !cursor || busy) return;
    const expected = scope;
    setBusy(true);
    setError("");
    try {
      const page = await request<{
        nodes: FleetNode[];
        next_cursor?: string | null;
      }>(`${path}&after=${encodeURIComponent(cursor)}`);
      if (!alive.current || current.current !== expected) return;
      setHistory({
        scope,
        nodes: [...nodes, ...page.nodes],
        cursor: page.next_cursor ?? null,
      });
    } catch (e) {
      if (alive.current && current.current === expected)
        setError(
          e instanceof Error ? e.message : "Workspaces could not be loaded.",
        );
    } finally {
      if (alive.current && current.current === expected) setBusy(false);
    }
  }
  return {
    nodes,
    loading: resource.loading,
    error: resource.error || error,
    refresh: resource.refresh,
    more,
    busy,
    hasMore: Boolean(cursor),
  };
}

function WorkspaceThreads({
  project,
  item,
  selected,
  search,
  archived,
  expanded,
  identity,
  onOpen,
}: {
  project: string;
  item: FleetNode;
  selected: string;
  search: string;
  archived: string;
  expanded: boolean;
  identity: string;
  onOpen: (thread: Thread) => void;
}) {
  const params = new URLSearchParams({
    node_id: item.node.node_id,
    archived,
    limit: "25",
  });
  if (search) params.set("query", search);
  const path = `${projectPath(project)}/threads?${params}`;
  const scope = JSON.stringify([identity, path]);
  const resource = useResource<{
    threads: Thread[];
    next_cursor?: string | null;
  }>(expanded ? path : null, 5000, identity);
  const [older, setOlder] = useState<{
    scope: string;
    threads: Thread[];
    cursor: string | null;
  }>({ scope, threads: [], cursor: null });
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const current = useRef(scope),
    alive = useRef(true);
  current.current = scope;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    setOlder({ scope, threads: [], cursor: null });
    setBusy(false);
    setError("");
  }, [scope]);
  const map = new Map<string, Thread>();
  if (older.scope === scope)
    for (const thread of older.threads) map.set(thread.thread_id, thread);
  for (const thread of resource.data?.threads ?? [])
    map.set(thread.thread_id, thread);
  const threads = resource.error
    ? []
    : [...map.values()]
        .filter(
          (thread) =>
            thread.project_id === project &&
            thread.node_id === item.node.node_id &&
            thread.archived === (archived === "true"),
        )
        .sort(
          (a, b) =>
            b.updated_at.localeCompare(a.updated_at) ||
            a.thread_id.localeCompare(b.thread_id),
        );
  const cursor =
    older.scope === scope && older.threads.length
      ? older.cursor
      : resource.data?.next_cursor;
  async function more() {
    if (!cursor || busy) return;
    const expected = scope;
    setBusy(true);
    setError("");
    try {
      const page = await request<{
        threads: Thread[];
        next_cursor?: string | null;
      }>(`${path}&after=${encodeURIComponent(cursor)}`);
      if (!alive.current || current.current !== expected) return;
      setOlder({
        scope,
        threads: [...threads, ...page.threads],
        cursor: page.next_cursor ?? null,
      });
    } catch (e) {
      if (alive.current && current.current === expected)
        setError(
          e instanceof Error
            ? e.message
            : "Older conversations could not be loaded.",
        );
    } finally {
      if (alive.current && current.current === expected) setBusy(false);
    }
  }
  if (!expanded) return null;
  return (
    <div className="host-workspace-threads">
      <LoadState {...resource} retry={resource.refresh} />
      <nav aria-label={`${workspaceName(item)} conversations`}>
        <SidebarMenu>
          {threads.map((thread) => (
            <SidebarMenuItem key={thread.thread_id}>
              <SidebarMenuSubButton
                asChild
                isActive={selected === thread.thread_id}
                className="host-thread-link"
              >
                <RouteLink
                  className={`host-thread-link ${selected === thread.thread_id ? "is-selected" : ""}`}
                  aria-current={
                    selected === thread.thread_id ? "page" : undefined
                  }
                  href={threadHref(project, thread.thread_id)}
                  onNavigate={() => onOpen(thread)}
                  title={thread.title}
                >
                  <WorkspaceSidebarThreadContent
                    title={thread.title || "Untitled conversation"}
                    metadata={
                      <span>
                        {thread.source === "runtime"
                          ? "Shared"
                          : "Conversation"}{" "}
                        · {new Date(thread.updated_at).toLocaleDateString()}
                      </span>
                    }
                    status={
                      !thread.can_continue ? (
                        <IconLock size={13} aria-label="View only" />
                      ) : undefined
                    }
                  />
                </RouteLink>
              </SidebarMenuSubButton>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </nav>
      {!resource.loading && !resource.error && !threads.length ? (
        <p className="host-sidebar-empty">
          {search ? "No matching threads" : "No threads yet"}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="host-sidebar-error">
          {error}
        </p>
      ) : null}
      {cursor ? (
        <Button
          variant="tertiary"
          className="host-history-more"
          disabled={busy}
          onClick={() => void more()}
        >
          {busy ? "Loading…" : "Older threads"}
        </Button>
      ) : null}
    </div>
  );
}

export function HostSidebar({
  project,
  host,
  nodes,
  agentId,
  selected,
  identity,
  canExecute,
  onAgent,
  onOpen,
  onNew,
  onBack,
  hasMore,
  onMore,
  moreBusy,
  error,
  view,
  onView,
}: {
  project: string;
  host: Host;
  nodes: FleetNode[];
  agentId: string;
  selected: string;
  identity: string;
  canExecute: boolean;
  onAgent: (id: string) => void;
  onOpen: (thread: Thread) => void;
  onNew: (id: string) => void;
  onBack: () => void;
  hasMore: boolean;
  onMore: () => void;
  moreBusy: boolean;
  error: string;
  view?: import("./routes").AgentView;
  onView?: (view: import("./routes").AgentView) => void;
}) {
  const [query, setQuery] = useState(""),
    [search, setSearch] = useState(""),
    [archived, setArchived] = useState("false");
  const [folding, setFolding] = useState<Record<string, boolean>>({});
  const [compact, setCompact] = useState(
    () => window.matchMedia?.("(max-width: 760px)").matches ?? false,
  );
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const media = window.matchMedia?.("(max-width: 760px)");
    if (!media) return;
    const update = () => setCompact(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), 250);
    return () => clearTimeout(timer);
  }, [query]);
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
  const items = nodes
    .filter(
      (item) =>
        item.node.project_id === project && item.node.host_id === host.host_id,
    )
    .sort(
      (a, b) =>
        workspaceName(a).localeCompare(workspaceName(b)) ||
        a.node.node_id.localeCompare(b.node.node_id),
    );
  return (
    <aside
      className="agent-sidebar host-sidebar"
      aria-label={`${host.label} workspaces`}
    >
      <SidebarHeader className="host-sidebar-header">
        <FoundationButton asChild variant="ghost" className="scope-fleet-back">
          <RouteLink href={globalHref("fleet", project)} onNavigate={onBack}>
            <IconArrowLeft size={16} aria-hidden="true" /> Fleet
          </RouteLink>
        </FoundationButton>
        <div className="host-sidebar-identity">
          <IconDeviceDesktop size={20} aria-hidden="true" />
          <div>
            <strong>{host.label}</strong>
            <span>{platformName(host.platform)}</span>
          </div>
        </div>
        <WorkspaceSidebarHeading />
        <WorkspaceSidebarSearch
          value={query}
          onChange={setQuery}
          inputRef={searchRef}
          shortcut={
            navigator.platform.toLowerCase().includes("mac") ? "⌘K" : "Ctrl K"
          }
          maxLength={256}
        />
        <div className="host-sidebar-filter">
          <span>On this host</span>
          <DropdownSelect
            aria-label="Conversation archive filter"
            value={archived}
            onChange={(event) => setArchived(event.target.value)}
          >
            <option value="false">Open</option>
            <option value="true">Archived</option>
          </DropdownSelect>
        </div>
      </SidebarHeader>
      <SidebarContent className="host-workspace-list">
        {items.map((item, index) => {
          const id = item.node.node_id,
            expanded =
              Boolean(search) ||
              (folding[id] ?? (!compact && (index < 4 || id === agentId)));
          const state = workspaceState(item);
          return (
            <Collapsible
              open={expanded}
              onOpenChange={(next) =>
                setFolding((current) => ({ ...current, [id]: next }))
              }
              className={`host-workspace-section ${agentId === id ? "is-active" : ""}`}
              key={id}
              aria-label={`${workspaceName(item)} workspace`}
            >
              <WorkspaceSidebarWorkspace
                className="host-workspace-row"
                identity={
                  <SidebarMenuButton
                    asChild
                    isActive={agentId === id}
                    className="shared-workspace-row-identity"
                  >
                    <RouteLink
                      href={agentHref(project, id)}
                      onNavigate={() => onAgent(id)}
                      aria-current={agentId === id ? "page" : undefined}
                    >
                      <IconFolder size={17} aria-hidden="true" />
                      <strong>{workspaceName(item)}</strong>
                    </RouteLink>
                  </SidebarMenuButton>
                }
                state={
                  <span
                    className="shared-workspace-readiness"
                    title={state}
                    aria-label={state}
                  >
                    <span
                      className={`dot ${state === "Ready" ? "live" : ""}`}
                    />
                  </span>
                }
                actions={
                  <>
                    <CollapsibleTrigger asChild>
                      <FoundationButton
                        variant="ghost"
                        size="icon"
                        className="scope-workspace-toggle"
                        aria-label={`${expanded ? "Collapse" : "Expand"} ${workspaceName(item)} threads`}
                      >
                        <IconChevronDown size={15} aria-hidden="true" />
                      </FoundationButton>
                    </CollapsibleTrigger>
                    <FoundationButton
                      variant="ghost"
                      size="icon"
                      className="scope-new-thread"
                      aria-label={`New conversation in ${workspaceName(item)}`}
                      disabled={!canExecute || state !== "Ready"}
                      onClick={() => onNew(id)}
                    >
                      <IconPencilPlus size={16} aria-hidden="true" />
                    </FoundationButton>
                  </>
                }
              />
              <CollapsibleContent>
                <WorkspaceThreads
                  project={project}
                  item={item}
                  selected={selected}
                  search={search}
                  archived={archived}
                  expanded={expanded}
                  identity={identity}
                  onOpen={onOpen}
                />
              </CollapsibleContent>
            </Collapsible>
          );
        })}
        {error ? (
          <p className="host-sidebar-error" role="alert">
            {error}
          </p>
        ) : null}
        {hasMore ? (
          <Button variant="tertiary" disabled={moreBusy} onClick={onMore}>
            {moreBusy ? "Loading…" : "More workspaces"}
          </Button>
        ) : null}
      </SidebarContent>
      {agentId && onView ? (
        <WorkspaceNavigation
          active={
            (
              Object.keys(
                WORKSPACE_VIEW_ROUTES,
              ) as (keyof typeof WORKSPACE_VIEW_ROUTES)[]
            ).find((key) => WORKSPACE_VIEW_ROUTES[key] === view) ?? "work"
          }
          renderItem={(item, children) => (
            <RouteLink
              className="sidebar-nav-item"
              href={agentHref(project, agentId, WORKSPACE_VIEW_ROUTES[item.id])}
              onNavigate={() => onView(WORKSPACE_VIEW_ROUTES[item.id])}
              aria-current={
                view === WORKSPACE_VIEW_ROUTES[item.id] ? "page" : undefined
              }
            >
              {children}
            </RouteLink>
          )}
        />
      ) : null}
    </aside>
  );
}

export function HostOverview({
  host,
  nodes,
  project,
  onAgent,
}: {
  host: Host;
  nodes: FleetNode[];
  project: string;
  onAgent: (id: string) => void;
}) {
  const connection = hostConnection(host, nodes);
  return (
    <section className="host-overview managed-settings-body">
      <header className="page-heading">
        <div>
          <p className="eyebrow">Host</p>
          <h1>{host.label}</h1>
          <p>
            {platformName(host.platform)} ·{" "}
            {host.deployment_kind === "desktop" ? "Desktop" : "CLI"}
          </p>
        </div>
        <span className="host-connection-status">
          <span className={`dot ${connection.ready ? "live" : ""}`} />
          {connection.label}
        </span>
      </header>
      <div className="host-workspace-welcome">
        <IconFolder size={34} aria-hidden="true" />
        <h2>Your workspaces</h2>
        <p>
          Choose a workspace on the left to open its threads or start a
          conversation.
        </p>
        <div className="host-workspace-shortcuts">
          {nodes
            .filter(
              (item) =>
                item.node.host_id === host.host_id &&
                item.node.project_id === project,
            )
            .map((item) => (
              <RouteLink
                className="ui-button ui-button--secondary"
                key={item.node.node_id}
                href={agentHref(project, item.node.node_id)}
                onNavigate={() => onAgent(item.node.node_id)}
              >
                <IconFolder size={17} aria-hidden="true" />
                {workspaceName(item)}
              </RouteLink>
            ))}
        </div>
      </div>
      <details className="host-diagnostics">
        <summary>Host details</summary>
        <dl>
          <dt>Host identity</dt>
          <dd className="mono">{host.host_id}</dd>
          <dt>Last contact</dt>
          <dd>
            {host.last_seen_at
              ? new Date(host.last_seen_at * 1000).toLocaleString()
              : "Not reported"}
          </dd>
        </dl>
      </details>
    </section>
  );
}
