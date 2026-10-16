import { useState } from "react";
import { Button, HostPlatformIcon, TextInput } from "@colossus/ui";
import {
  IconArrowUpRight,
  IconChevronRight,
  IconPlus,
  IconRefresh,
} from "@tabler/icons-react";
import type {
  FleetNode,
  Host,
  Permission,
  Task,
  Thread,
} from "@colossus/control-plane-client";
import { messageOf, type MobileClient } from "./client";
import { Conversation } from "./Conversation";
import { usePoll } from "./usePoll";

export type Tab = "work" | "fleet" | "inbox" | "settings";
export function Workspace({
  client,
  project,
  permissions,
  tab,
  onLocked,
}: {
  client: MobileClient;
  project: string;
  permissions: Permission[];
  tab: Tab;
  onLocked: (locked: boolean) => void;
}) {
  const [selection, setSelection] = useState<Thread | "new" | null>(null);
  const [query, setQuery] = useState("");
  const [browserError, setBrowserError] = useState("");
  const [moreThreads, setMoreThreads] = useState<Thread[]>([]),
    [moreNodes, setMoreNodes] = useState<FleetNode[]>([]);
  const [pageBusy, setPageBusy] = useState(false);
  const fleet = usePoll<{ nodes: FleetNode[]; next_cursor?: string | null }>(
    client,
    { kind: "fleet", project },
  );
  const threads = usePoll<{ threads: Thread[]; next_cursor?: string | null }>(
    client,
    { kind: "threads", project },
  );
  const tasks = usePoll<{ tasks: Task[] }>(client, { kind: "tasks", project });
  const nodes = [
    ...new Map(
      [...(fleet.value?.nodes ?? []), ...moreNodes].map((n) => [
        n.node.node_id,
        n,
      ]),
    ).values(),
  ];
  const conversations = [
    ...new Map(
      [...(threads.value?.threads ?? []), ...moreThreads].map((t) => [
        t.thread_id,
        t,
      ]),
    ).values(),
  ];
  const [threadCursor, setThreadCursor] = useState<string | null | undefined>(
      undefined,
    ),
    [nodeCursor, setNodeCursor] = useState<string | null | undefined>(
      undefined,
    );
  const hostPage = usePoll<{ hosts: Host[] }>(client, {
    kind: "hosts",
    project,
  });
  const hosts = new Map(
    hostPage.value?.hosts.map((host) => [host.host_id, host]) ?? [],
  );
  async function more(kind: "threads" | "fleet") {
    const after =
      kind === "threads"
        ? (threadCursor ?? threads.value?.next_cursor)
        : (nodeCursor ?? fleet.value?.next_cursor);
    if (!after || pageBusy) return;
    setPageBusy(true);
    try {
      if (kind === "threads") {
        const page = await client.request<{
          threads: Thread[];
          next_cursor: string | null;
        }>({ kind, project, after });
        setMoreThreads((all) => [...all, ...page.threads]);
        setThreadCursor(page.next_cursor ?? null);
      } else {
        const page = await client.request<{
          nodes: FleetNode[];
          next_cursor: string | null;
        }>({ kind, project, after });
        setMoreNodes((all) => [...all, ...page.nodes]);
        setNodeCursor(page.next_cursor ?? null);
      }
    } catch (failure) {
      setBrowserError(messageOf(failure));
    } finally {
      setPageBusy(false);
    }
  }
  if (selection !== null && tab === "work")
    return (
      <Conversation
        key={selection === "new" ? "new" : selection.thread_id}
        client={client}
        project={project}
        thread={selection === "new" ? null : selection}
        nodes={nodes}
        permissions={permissions}
        onBack={() => setSelection(null)}
        onCreated={setSelection}
        onLocked={onLocked}
        hosts={hostPage.value?.hosts ?? []}
      />
    );
  const pending =
    tasks.value?.tasks.filter((t) =>
      t.snapshot?.pending_interactions.some((i) => i.status === "pending"),
    ) ?? [];
  const platform = (node: FleetNode | undefined) => {
    // Only authenticated host metadata establishes a platform.
    return hosts.get(node?.node.host_id ?? "")?.platform ?? "unknown";
  };
  return (
    <main className="mobile-workspace">
      <header className="mobile-page-heading">
        <div>
          <h1>
            {tab === "fleet" ? "Fleet" : tab === "inbox" ? "Inbox" : "Work"}
          </h1>
          <p className="mobile-muted">
            {tab === "fleet"
              ? "Your connected hosts and workspaces."
              : tab === "inbox"
                ? "Approvals and questions from your tasks."
                : "Continue work across your hosts."}
          </p>
        </div>
        <Button
          variant="tertiary"
          aria-label="Refresh"
          onClick={() => {
            fleet.refresh();
            hostPage.refresh();
            threads.refresh();
            tasks.refresh();
          }}
        >
          <IconRefresh size={20} />
        </Button>
      </header>
      {(fleet.error ||
        hostPage.error ||
        threads.error ||
        tasks.error ||
        browserError) && (
        <p role="alert" className="mobile-error">
          {browserError ||
            fleet.error ||
            hostPage.error ||
            threads.error ||
            tasks.error}
        </p>
      )}
      {tab === "work" && (
        <>
          <Button
            className="mobile-new-task"
            variant="primary"
            disabled={
              !permissions.includes("execute") ||
              !nodes.some((n) => n.presence?.ready && !n.node.revoked)
            }
            onClick={() => setSelection("new")}
          >
            <IconPlus size={20} />
            New task
          </Button>
          {pending[0] && (
            <div className="mobile-attention">
              <h2>Needs your review</h2>
              <p>
                {pending[0].snapshot?.pending_interactions[0]?.content.approval
                  ? "Review a requested action"
                  : "Answer an agent’s question"}
              </p>
              <Button
                onClick={() =>
                  void client
                    .openBrowser(project, pending[0]!.task_id)
                    .catch((failure) => setBrowserError(messageOf(failure)))
                }
              >
                Review in Control Plane <IconArrowUpRight size={16} />
              </Button>
            </div>
          )}
          <div className="mobile-section-heading">
            <h2>Recent work</h2>
          </div>
          <label className="shared-sr-only" htmlFor="work-search">
            Search loaded conversations
          </label>
          <TextInput
            id="work-search"
            placeholder="Search conversations"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          {!threads.value && !threads.error && (
            <p role="status">Loading work…</p>
          )}
          <div className="mobile-rows">
            {conversations
              .filter(
                (t) =>
                  !t.archived &&
                  t.title
                    .toLocaleLowerCase()
                    .includes(query.toLocaleLowerCase()),
              )
              .map((t) => {
                const node = nodes.find((n) => n.node.node_id === t.node_id);
                return (
                  <button
                    type="button"
                    className="mobile-row"
                    key={t.thread_id}
                    onClick={() => setSelection(t)}
                  >
                    <HostPlatformIcon platform={platform(node)} size={25} />
                    <span>
                      <strong>{t.title}</strong>
                      <small>
                        {[
                          hosts.get(node?.node.host_id ?? "")?.label ??
                            node?.node.label,
                          node?.node.workspace_label,
                        ]
                          .filter(Boolean)
                          .join(" · ") || "Assigned workspace"}
                      </small>
                      <small>
                        {t.active_task_id
                          ? "Working"
                          : t.sync_status === "incomplete"
                            ? "History incomplete"
                            : "Open conversation"}
                      </small>
                    </span>
                    <IconChevronRight size={18} />
                  </button>
                );
              })}
          </div>
          {threads.value && !conversations.length && (
            <div className="mobile-empty">
              <h2>Your work starts here</h2>
              <p>
                {nodes.length
                  ? "Start a task on a connected workspace."
                  : "Enroll a workspace in Control Plane to get started."}
              </p>
            </div>
          )}
          {(threadCursor === undefined
            ? threads.value?.next_cursor
            : threadCursor) && (
            <Button disabled={pageBusy} onClick={() => void more("threads")}>
              Load older conversations
            </Button>
          )}
        </>
      )}
      {tab === "fleet" && (
        <>
          <div className="mobile-rows">
            {nodes.map((n) => (
              <div className="mobile-row" key={n.node.node_id}>
                <HostPlatformIcon platform={platform(n)} size={28} />
                <span>
                  <strong>{n.node.label}</strong>
                  <small>{n.node.workspace_label ?? "Workspace"}</small>
                  <small
                    className={
                      n.presence?.ready && !n.node.revoked
                        ? "mobile-connected"
                        : ""
                    }
                  >
                    {n.node.revoked
                      ? "Revoked"
                      : n.presence?.ready
                        ? "Connected"
                        : "Offline"}
                  </small>
                </span>
              </div>
            ))}
          </div>
          {fleet.value && !nodes.length && (
            <p className="mobile-empty">
              No workspaces are enrolled in this project.
            </p>
          )}
          {(nodeCursor === undefined
            ? fleet.value?.next_cursor
            : nodeCursor) && (
            <Button disabled={pageBusy} onClick={() => void more("fleet")}>
              Load more workspaces
            </Button>
          )}
          <p className="mobile-caption">
            Workspace enrollment and permissions are managed in Control Plane.
          </p>
        </>
      )}
      {tab === "inbox" && (
        <>
          {!tasks.value && !tasks.error && (
            <p role="status">Loading requests…</p>
          )}
          {pending.map((t) => (
            <section className="mobile-attention" key={t.task_id}>
              <h2>{t.snapshot?.run.title || "Agent request"}</h2>
              {t.snapshot?.pending_interactions
                .filter((i) => i.status === "pending")
                .map((i) => (
                  <p key={i.interaction_id}>
                    {i.content.approval?.reason ??
                      i.content.user_prompt?.question ??
                      "Your input is needed."}
                  </p>
                ))}
              <Button
                onClick={() =>
                  void client
                    .openBrowser(project, t.task_id)
                    .catch((failure) => setBrowserError(messageOf(failure)))
                }
              >
                Review in Control Plane <IconArrowUpRight size={16} />
              </Button>
            </section>
          ))}
          {tasks.value && !pending.length && (
            <div className="mobile-empty">
              <h2>You’re all caught up</h2>
              <p>No pending requests in the loaded task page.</p>
            </div>
          )}
          <p className="mobile-caption">
            Showing the latest task page. Open Control Plane for the full task
            inventory.
          </p>
        </>
      )}
    </main>
  );
}
