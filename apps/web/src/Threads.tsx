import { Badge } from "@colossus/ui/components/ui/badge";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@colossus/ui/components/ui/table";
import {
  IconArrowRight,
  IconChevronLeft,
  IconChevronRight,
  IconMessageCircle,
  IconPlus,
  IconSearch,
  IconServer,
} from "@tabler/icons-react";
import {
  projectPath,
  request,
  type FleetNode,
  type Permission,
  type Thread,
  type Task,
} from "./api";
import { RunComposer, type RunRequest } from "./RunComposer";

export function Threads({
  project,
  nodes,
  permissions,
  agentId,
  onAgentChange,
  onOpen,
  onError,
  onFleet,
}: {
  project: string;
  nodes: FleetNode[];
  permissions: Permission[];
  agentId: string;
  onAgentChange: (id: string) => void;
  onOpen: (thread: Thread) => void;
  onError: (error: string) => void;
  onFleet: () => void;
}) {
  const [threads, setThreads] = useState<Thread[]>([]),
    [query, setQuery] = useState(""),
    [search, setSearch] = useState(""),
    [archive, setArchive] = useState("false"),
    [cursors, setCursors] = useState<string[]>([""]),
    [next, setNext] = useState<string | null>(null),
    [loading, setLoading] = useState(true),
    [creating, setCreating] = useState(false),
    [busy, setBusy] = useState(false),
    [targetId, setTargetId] = useState(agentId);
  const alive = useRef(true);
  const heading = useRef<HTMLHeadingElement>(null),
    initialAgent = useRef(agentId);
  useEffect(() => {
    if (initialAgent.current) heading.current?.focus();
  }, []);
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
  useEffect(() => {
    setCursors([""]);
  }, [search, agentId, archive]);
  useEffect(() => {
    setTargetId(
      (current) =>
        agentId ||
        (nodes.some(
          (item) => item.node.node_id === current && !item.node.revoked,
        )
          ? current
          : nodes.find((item) => !item.node.revoked && item.presence?.ready)
              ?.node.node_id ||
            nodes.find((item) => !item.node.revoked)?.node.node_id ||
            ""),
    );
  }, [agentId, nodes]);
  const cursor = cursors[cursors.length - 1] ?? "";
  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      const params = new URLSearchParams({ limit: "25", archived: archive });
      if (agentId) params.set("node_id", agentId);
      if (search) params.set("query", search);
      if (cursor) params.set("after", cursor);
      try {
        const result = await request<{
          threads: Thread[];
          next_cursor?: string | null;
        }>(`${projectPath(project)}/threads?${params}`, undefined, signal);
        if (signal?.aborted || !alive.current) return;
        setThreads(result.threads);
        setNext(
          result.next_cursor ??
            (result.threads.length === 25
              ? result.threads.at(-1)!.thread_id
              : null),
        );
        setLoading(false);
      } catch (error) {
        if (!signal?.aborted && alive.current) {
          setLoading(false);
          onError(
            error instanceof Error
              ? error.message
              : "Threads could not be loaded.",
          );
        }
      }
    },
    [project, agentId, search, archive, cursor, onError],
  );
  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
    setLoading(true);
    const poll = async () => {
      if (pending) return;
      pending = true;
      try {
        await refresh(abort.signal);
      } finally {
        pending = false;
      }
    };
    void poll();
    const timer = setInterval(() => {
      if (!document.hidden) void poll();
    }, 3000);
    return () => {
      abort.abort();
      clearInterval(timer);
    };
  }, [refresh]);
  async function create(runRequest: RunRequest) {
    setBusy(true);
    try {
      const result = await request<{ thread: Thread; task: Task }>(
        `${projectPath(project)}/threads`,
        { node_id: targetId, request: runRequest },
      );
      if (!alive.current) return false;
      setCreating(false);
      onOpen(result.thread);
      return true;
    } catch (error) {
      if (alive.current)
        onError(
          error instanceof Error
            ? error.message
            : "Thread creation failed. Retry to reconcile the same request.",
        );
      return false;
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  const labels = new Map(
    nodes.map((item) => [item.node.node_id, item.node.label]),
  );
  const selectedAgent = nodes.find((item) => item.node.node_id === agentId);
  return (
    <section
      className="managed-settings-body catalog-settings threads-settings"
      aria-labelledby="threads-heading"
    >
      <header className="catalog-heading">
        <div>
          <h2 id="threads-heading" ref={heading} tabIndex={-1}>
            Threads
          </h2>
          <p>
            {selectedAgent
              ? `Conversations on ${selectedAgent.node.label}${selectedAgent.node.workspace_label ? ` · ${selectedAgent.node.workspace_label}` : ""}.`
              : "Open a conversation, follow live work, or continue a saved session."}
          </p>
        </div>
        <Button
          variant="primary"
          disabled={
            !permissions.includes("execute") ||
            !nodes.some((item) => !item.node.revoked)
          }
          onClick={() => setCreating((value) => !value)}
        >
          <IconPlus size={16} aria-hidden="true" />
          {creating ? "Close composer" : "New thread"}
        </Button>
      </header>
      {selectedAgent && !selectedAgent.presence?.ready ? (
        <p className="sync-notice" role="status">
          <IconServer size={16} aria-hidden="true" />
          Agent offline. Synced conversations remain available; new messages
          queue until it reconnects.
        </p>
      ) : null}
      {creating ? (
        <RunComposer
          nodes={nodes}
          nodeId={targetId}
          onNodeChange={setTargetId}
          busy={busy}
          label="Start a conversation"
          action="Start thread"
          disabled={!permissions.includes("execute")}
          onSubmit={create}
        />
      ) : null}
      <div className="thread-toolbar">
        <label className="thread-search">
          <IconSearch size={16} aria-hidden="true" />
          <span className="sr-only">Search threads</span>
          <TextInput
            aria-label="Search threads"
            placeholder="Search threads…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <DropdownSelect
          aria-label="Filter threads by agent"
          value={agentId}
          onChange={(event) => onAgentChange(event.target.value)}
        >
          <option value="">All agents</option>
          {nodes.map((item) => (
            <option key={item.node.node_id} value={item.node.node_id}>
              {item.node.label}
            </option>
          ))}
        </DropdownSelect>
        <DropdownSelect
          aria-label="Thread archive filter"
          value={archive}
          onChange={(event) => setArchive(event.target.value)}
        >
          <option value="false">Open threads</option>
          <option value="true">Archived threads</option>
        </DropdownSelect>
      </div>
      <div className="thread-table" aria-busy={loading}>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead scope="col">Conversation</TableHead>
              <TableHead scope="col">Agent / workspace</TableHead>
              <TableHead scope="col">History</TableHead>
              <TableHead scope="col">Updated</TableHead>
              <TableHead scope="col">
                <span className="sr-only">Open thread</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {threads.map((thread) => (
              <TableRow key={thread.thread_id}>
                <TableHead scope="row" data-label="Conversation">
                  <div className="catalog-identity">
                    <span className="resource-icon">
                      <IconMessageCircle size={16} aria-hidden="true" />
                    </span>
                    <div>
                      <button
                        className="catalog-name"
                        type="button"
                        onClick={() => onOpen(thread)}
                      >
                        {thread.title || "Untitled thread"}
                      </button>
                      <small>
                        {thread.source === "runtime"
                          ? "Shared local session"
                          : "Control Plane conversation"}
                        {thread.active_task_id ? " · Active work" : ""}
                      </small>
                    </div>
                  </div>
                </TableHead>
                <TableCell data-label="Agent / workspace">
                  <div className="thread-agent-cell">
                    {labels.get(thread.node_id) ?? thread.node_id.slice(0, 8)}
                    {nodes.find((item) => item.node.node_id === thread.node_id)
                      ?.node.workspace_label ? (
                      <small className="thread-cell-note">
                        {
                          nodes.find(
                            (item) => item.node.node_id === thread.node_id,
                          )?.node.workspace_label
                        }
                      </small>
                    ) : null}
                  </div>
                </TableCell>
                <TableCell data-label="History">
                  <Badge
                    className={`status ${thread.sync_status === "current" ? "status-completed" : "status-waiting"}`}
                  >
                    {thread.sync_status === "current"
                      ? "Synced"
                      : "Sync incomplete"}
                  </Badge>
                </TableCell>
                <TableCell data-label="Updated">
                  <time
                    dateTime={thread.updated_at}
                    title={new Date(thread.updated_at).toLocaleString()}
                  >
                    {new Date(thread.updated_at).toLocaleDateString(undefined, {
                      month: "short",
                      day: "numeric",
                    })}
                  </time>
                </TableCell>
                <TableCell className="thread-row-action">
                  <button
                    type="button"
                    className="ui-icon-button"
                    aria-label={`Open thread: ${thread.title}`}
                    onClick={() => onOpen(thread)}
                  >
                    <IconArrowRight size={16} aria-hidden="true" />
                  </button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
      {!threads.length ? (
        <div className="empty-state">
          <IconMessageCircle size={28} aria-hidden="true" />
          <h3>
            {loading
              ? "Loading conversations…"
              : search
                ? "No matching threads"
                : archive === "true"
                  ? "No archived threads"
                  : "No conversations yet"}
          </h3>
          <p>
            {search
              ? "Try a different title or agent."
              : "Control Plane conversations and explicitly shared local sessions appear here."}
          </p>
          {!nodes.length ? (
            <Button onClick={onFleet}>Set up an agent</Button>
          ) : null}
        </div>
      ) : null}
      <div className="thread-pagination">
        <span>
          {threads.length} conversations · Page {cursors.length}
        </span>
        <Button
          disabled={loading || cursors.length === 1}
          onClick={() => setCursors((value) => value.slice(0, -1))}
        >
          <IconChevronLeft size={16} aria-hidden="true" />
          Previous
        </Button>
        <Button
          disabled={loading || !next}
          onClick={() => {
            if (next) setCursors((value) => [...value, next]);
          }}
        >
          Next
          <IconChevronRight size={16} aria-hidden="true" />
        </Button>
      </div>
    </section>
  );
}
