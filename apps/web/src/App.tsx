import { useCallback, useEffect, useRef, useState } from "react";
import { ComposerInput, DropdownSelect } from "@colossus/ui";
import {
  IconActivity,
  IconArrowRight,
  IconCloud,
  IconServer,
  IconListDetails,
  IconLogout,
  IconPlus,
  IconSearch,
  IconShieldLock,
  IconLoader2,
  IconRefresh,
  IconMoon,
  IconSun,
  IconSend2,
} from "@tabler/icons-react";
import {
  ApiFailure,
  request,
  projectPath,
  taskTitle,
  taskStatus,
  statusLabel,
  terminalStatuses,
  type Membership,
  type FleetNode,
  type Task,
} from "./api";
import { Fleet } from "./Fleet";
import { TaskDetail } from "./TaskDetail";
export function App() {
  const [memberships, setMemberships] = useState<Membership[] | null>(null),
    [auth, setAuth] = useState<"loading" | "signed_out" | "ready">("loading"),
    [project, setProject] = useState(""),
    [surface, setSurface] = useState<"tasks" | "fleet">("tasks"),
    [nodes, setNodes] = useState<FleetNode[]>([]),
    [tasks, setTasks] = useState<Task[]>([]),
    [taskPages, setTaskPages] = useState(1),
    [nodePages, setNodePages] = useState(1),
    [selected, setSelected] = useState<Task | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(true),
    [draft, setDraft] = useState(""),
    [node, setNode] = useState(""),
    [mode, setMode] = useState("execute"),
    [role, setRole] = useState("primary"),
    [busy, setBusy] = useState(false),
    [query, setQuery] = useState(""),
    [filter, setFilter] = useState("all"),
    [light, setLight] = useState(
      () => localStorage.getItem("colossus-cloud-theme") === "light",
    );
  const composer = useRef<HTMLTextAreaElement>(null),
    attempt = useRef<{ signature: string; key: string } | null>(null);
  const currentProject = useRef(project);
  currentProject.current = project;
  useEffect(() => {
    document.documentElement.dataset.theme = light ? "light" : "dark";
    document.documentElement.dataset.palette = "neutral";
    localStorage.setItem("colossus-cloud-theme", light ? "light" : "dark");
  }, [light]);
  useEffect(() => {
    const abort = new AbortController();
    void request<{ memberships: Membership[] }>(
      "/api/me",
      undefined,
      abort.signal,
    )
      .then((response) => {
        setMemberships(response.memberships);
        setProject(response.memberships[0]?.project_id ?? "");
        setAuth("ready");
      })
      .catch((error) => {
        if (!abort.signal.aborted) {
          if (error instanceof ApiFailure && error.status === 403)
            setAuth("signed_out");
          else {
            setError(
              error instanceof Error ? error.message : "Sign-in unavailable.",
            );
            setAuth("signed_out");
          }
        }
      });
    return () => abort.abort();
  }, []);
  const membership = memberships?.find(
      (member) => member.project_id === project,
    ),
    permissions = membership?.permissions ?? [];
  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      if (!project) return;
      async function pages<T>(
        kind: "tasks" | "nodes",
        count: number,
        id: (item: T) => string,
      ) {
        const items: T[] = [];
        let after = "";
        for (let page = 0; page < count; page++) {
          const response = await request<Record<string, T[]>>(
            `${projectPath(project)}/${kind}?limit=100${after ? `&after=${encodeURIComponent(after)}` : ""}`,
            undefined,
            signal,
          );
          const batch = response[kind] ?? [];
          items.push(...batch);
          if (batch.length < 100) break;
          after = id(batch[batch.length - 1]!);
        }
        return items;
      }
      try {
        const [fleet, work] = await Promise.all([
          pages<FleetNode>("nodes", nodePages, (item) => item.node.node_id),
          pages<Task>("tasks", taskPages, (item) => item.task_id),
        ]);
        if (signal?.aborted || currentProject.current !== project) return;
        setNodes(fleet);
        setTasks(work);
        setLoading(false);
      } catch (error) {
        if (!signal?.aborted && currentProject.current === project) {
          setLoading(false);
          if (error instanceof ApiFailure && error.status === 403)
            setAuth("signed_out");
          setError(
            error instanceof Error
              ? error.message
              : "Control plane unavailable.",
          );
        }
      }
    },
    [project, taskPages, nodePages],
  );
  useEffect(() => {
    if (auth !== "ready") return;
    const abort = new AbortController();
    let refreshing = false;
    const poll = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        await refresh(abort.signal);
      } finally {
        refreshing = false;
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
  }, [refresh, auth]);
  useEffect(() => {
    setTasks([]);
    setNodes([]);
    setSelected(null);
    setNode("");
    setLoading(true);
    setTaskPages(1);
    setNodePages(1);
  }, [project]);
  useEffect(() => {
    if (
      !node ||
      !nodes.some((fleet) => fleet.node.node_id === node && !fleet.node.revoked)
    ) {
      setNode(
        (
          nodes.find((fleet) => !fleet.node.revoked && fleet.presence?.ready) ??
          nodes.find((fleet) => !fleet.node.revoked)
        )?.node.node_id ?? "",
      );
    }
  }, [nodes, node]);
  const target = nodes.find((fleet) => fleet.node.node_id === node);
  useEffect(() => {
    if (target && !target.node.roles.includes(role))
      setRole(target.node.roles[0] ?? "primary");
  }, [target, role]);
  async function create() {
    if (!draft.trim() || !node || busy) return;
    setBusy(true);
    setError("");
    const signature = JSON.stringify({ draft, node, mode, role });
    if (attempt.current?.signature !== signature)
      attempt.current = { signature, key: crypto.randomUUID() };
    try {
      const response = await request<{ task: Task }>(
        `${projectPath(project)}/tasks`,
        {
          node_id: node,
          request: {
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
          },
        },
      );
      if (currentProject.current !== project) return;
      setDraft("");
      attempt.current = null;
      setSelected(response.task);
      void refresh();
    } catch (error) {
      if (currentProject.current === project)
        setError(
          error instanceof Error
            ? error.message
            : "Task submission failed. Retry to reconcile the same task.",
        );
    } finally {
      setBusy(false);
    }
  }
  async function signOut() {
    try {
      await request("/auth/logout", {});
      setAuth("signed_out");
      setProject("");
      setMemberships(null);
      setNodes([]);
      setTasks([]);
      setSelected(null);
    } catch (error) {
      setError(error instanceof Error ? error.message : "Sign-out failed.");
    }
  }
  if (auth === "loading")
    return (
      <div className="startup">
        <IconCloud size={38} />
        <IconLoader2 size={22} className="spin" />
        <p>Connecting to Colossus Cloud…</p>
      </div>
    );
  if (auth === "signed_out")
    return (
      <main className="sign-in">
        <div className="sign-in-art" aria-hidden="true">
          <div className="orbit orbit-one" />
          <div className="orbit orbit-two" />
          <div className="orbit orbit-three" />
          <div className="cloud-mark">
            <IconCloud size={52} />
          </div>
          <div className="orbit-node node-one">
            <IconServer size={22} />
          </div>
          <div className="orbit-node node-two">
            <IconActivity size={22} />
          </div>
          <div className="orbit-node node-three">
            <IconShieldLock size={22} />
          </div>
        </div>
        <div className="sign-in-copy">
          <div className="brand">
            <IconCloud size={24} />
            <strong>Colossus</strong>
            <span>CLOUD</span>
          </div>
          <p className="eyebrow">YOUR RUNTIMES. ONE CONTROL PLANE.</p>
          <h1>
            Work reaches <br />
            beyond one machine.
          </h1>
          <p>
            Connect your runtimes, start agent tasks, and follow every step from
            one shared workspace.
          </p>
          {error && (
            <div className="alert" role="alert">
              {error}
            </div>
          )}
          <a className="primary-link" href="/auth/login">
            Sign in with your organization <IconArrowRight size={18} />
          </a>
          <div className="sign-in-foot">
            <IconShieldLock size={16} />
            <span>Organization sign-in · Explicit runtime authority</span>
          </div>
        </div>
      </main>
    );
  const live = nodes.filter(
      (fleet) => fleet.presence?.ready && !fleet.node.revoked,
    ).length,
    active = tasks.filter(
      (task) => !terminalStatuses.has(taskStatus(task)),
    ).length;
  const shown = tasks
    .filter(
      (task) =>
        (filter === "all" ||
          (filter === "active"
            ? !terminalStatuses.has(taskStatus(task))
            : terminalStatuses.has(taskStatus(task)))) &&
        taskTitle(task).toLowerCase().includes(query.toLowerCase()),
    )
    .sort((a, b) =>
      (b.snapshot?.run.created_at ?? b.task_id).localeCompare(
        a.snapshot?.run.created_at ?? a.task_id,
      ),
    );
  return (
    <div className="shell">
      <aside className="rail">
        <a className="brand" href="/" aria-label="Colossus Cloud home">
          <IconCloud size={25} />
          <strong>Colossus</strong>
          <span>CLOUD</span>
        </a>
        <div className="project-picker">
          <span className="eyebrow">PROJECT</span>
          <DropdownSelect
            aria-label="Select project"
            value={project}
            onChange={(event) => {
              setSelected(null);
              setNodes([]);
              setTasks([]);
              setNode("");
              setError("");
              setProject(event.target.value);
            }}
          >
            {(memberships ?? []).map((member) => (
              <option key={member.project_id} value={member.project_id}>
                {member.project_id}
              </option>
            ))}
          </DropdownSelect>
        </div>
        <nav aria-label="Main navigation">
          <button
            className={surface === "tasks" ? "nav-active" : ""}
            onClick={() => {
              setSurface("tasks");
              setSelected(null);
            }}
          >
            <IconListDetails size={19} />
            Tasks<span className="nav-count">{active || ""}</span>
          </button>
          <button
            className={surface === "fleet" ? "nav-active" : ""}
            onClick={() => {
              setSurface("fleet");
              setSelected(null);
            }}
          >
            <IconServer size={19} />
            Runtime fleet
            <span className="nav-count">
              {live}/{nodes.filter((fleet) => !fleet.node.revoked).length}
            </span>
          </button>
        </nav>
        <div className="rail-bottom">
          <div className="authority-note">
            <IconShieldLock size={19} />
            <div>
              <strong>Local authority, always</strong>
              <p>Project access and runtime grants are enforced together.</p>
            </div>
          </div>
          <div className="rail-footer">
            <button
              className="icon-button"
              aria-label={
                light ? "Switch to dark theme" : "Switch to light theme"
              }
              onClick={() => setLight(!light)}
            >
              {light ? <IconMoon size={18} /> : <IconSun size={18} />}
            </button>
            <button className="text-button" onClick={() => void signOut()}>
              <IconLogout size={17} />
              Sign out
            </button>
          </div>
        </div>
      </aside>
      <div className="workspace">
        <header className="topbar">
          <div>
            <span className="muted">{project}</span>
            <span className="slash">/</span>
            <strong>
              {selected
                ? "Task detail"
                : surface === "fleet"
                  ? "Runtime fleet"
                  : "Tasks"}
            </strong>
          </div>
          <div className="connection-indicator">
            <span className={`dot ${live ? "live" : ""}`} />
            {live} runtime{live === 1 ? "" : "s"} online
            <button
              className="icon-button"
              aria-label="Refresh project"
              onClick={() => void refresh()}
            >
              <IconRefresh size={16} />
            </button>
          </div>
        </header>
        <main className="content" id="main-content">
          {error && (
            <div role="alert" className="alert">
              {error}
              <button aria-label="Dismiss error" onClick={() => setError("")}>
                ×
              </button>
            </div>
          )}
          {selected ? (
            <TaskDetail
              key={`${project}:${selected.task_id}`}
              initial={selected}
              project={project}
              permissions={permissions}
              nodeLabel={
                nodes.find((fleet) => fleet.node.node_id === selected.node_id)
                  ?.node.label ?? selected.node_id
              }
              onBack={() => setSelected(null)}
            />
          ) : surface === "fleet" ? (
            <Fleet
              key={project}
              nodes={nodes}
              project={project}
              permissions={permissions}
              onRefresh={() => void refresh()}
              onError={setError}
              hasMore={nodes.length === nodePages * 100}
              onMore={() => setNodePages((pages) => pages + 1)}
            />
          ) : (
            <>
              <div className="section-heading">
                <div>
                  <div className="eyebrow">PROJECT WORKSPACE</div>
                  <h1>Your agent work, in motion.</h1>
                  <p>
                    Start a task on any enrolled runtime and follow it through.
                  </p>
                </div>
                <button
                  onClick={() => composer.current?.focus()}
                  disabled={!permissions.includes("execute") || !node}
                >
                  <IconPlus size={17} />
                  New task
                </button>
              </div>
              <div className="stats-strip">
                <div>
                  <IconActivity size={20} />
                  <span>
                    Active tasks<strong>{active}</strong>
                  </span>
                </div>
                <div>
                  <IconServer size={20} />
                  <span>
                    Runtimes online
                    <strong>
                      {live}
                      <small>
                        {" "}
                        / {nodes.filter((fleet) => !fleet.node.revoked).length}
                      </small>
                    </strong>
                  </span>
                </div>
                <div>
                  <IconShieldLock size={20} />
                  <span>
                    Project access
                    <strong className="stat-text">
                      {permissions.includes("administer")
                        ? "Administrator"
                        : permissions.includes("execute")
                          ? "Contributor"
                          : "Observer"}
                    </strong>
                  </span>
                </div>
              </div>
              <section
                className="new-task-panel"
                aria-label="Create an agent task"
              >
                <ComposerInput
                  ref={composer}
                  aria-label="Describe your task"
                  placeholder={
                    node
                      ? "What should your agent work on?"
                      : "Enroll a runtime to start your first task…"
                  }
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  disabled={busy || !node || !permissions.includes("execute")}
                  onKeyDown={(event) => {
                    if (
                      (event.metaKey || event.ctrlKey) &&
                      event.key === "Enter"
                    ) {
                      event.preventDefault();
                      void create();
                    }
                  }}
                />
                <div className="composer-controls">
                  <DropdownSelect
                    aria-label="Execution runtime"
                    value={node}
                    onChange={(event) => setNode(event.target.value)}
                    disabled={busy}
                  >
                    {nodes
                      .filter((fleet) => !fleet.node.revoked)
                      .map((fleet) => (
                        <option
                          key={fleet.node.node_id}
                          value={fleet.node.node_id}
                        >
                          {fleet.node.label}
                          {fleet.presence?.ready ? "" : " · Offline"}
                        </option>
                      ))}
                  </DropdownSelect>
                  <DropdownSelect
                    aria-label="Run mode"
                    value={mode}
                    onChange={(event) => setMode(event.target.value)}
                    disabled={busy}
                  >
                    <option value="execute">Execute</option>
                    <option value="plan">Plan</option>
                  </DropdownSelect>
                  <DropdownSelect
                    aria-label="Agent role"
                    value={role}
                    onChange={(event) => setRole(event.target.value)}
                    disabled={busy}
                  >
                    {(target?.node.roles ?? []).map((role) => (
                      <option key={role} value={role}>
                        {role}
                      </option>
                    ))}
                  </DropdownSelect>
                  <span className="composer-hint">⌘ / Ctrl ↵</span>
                  <button
                    aria-label="Start task"
                    disabled={
                      busy ||
                      !draft.trim() ||
                      !node ||
                      !permissions.includes("execute")
                    }
                    onClick={() => void create()}
                  >
                    {busy ? (
                      <IconLoader2 size={18} className="spin" />
                    ) : (
                      <IconSend2 size={18} />
                    )}
                    <span>{busy ? "Starting…" : "Start task"}</span>
                  </button>
                </div>
              </section>
              <section className="tasks-panel">
                <div className="tasks-toolbar">
                  <div className="filter-group" aria-label="Filter tasks">
                    {["all", "active", "finished"].map((value) => (
                      <button
                        key={value}
                        className={filter === value ? "selected" : ""}
                        aria-pressed={filter === value}
                        onClick={() => setFilter(value)}
                      >
                        {value === "all" ? "All tasks" : statusLabel(value)}
                      </button>
                    ))}
                  </div>
                  <div className="search">
                    <IconSearch size={16} />
                    <input
                      aria-label="Search tasks"
                      placeholder="Search tasks…"
                      value={query}
                      onChange={(event) => setQuery(event.target.value)}
                    />
                  </div>
                </div>
                {loading ? (
                  <div className="empty-state compact">
                    <IconLoader2 size={24} className="spin" />
                    <p>Loading project tasks…</p>
                  </div>
                ) : shown.length === 0 ? (
                  <div className="empty-state">
                    <div className="empty-icon">
                      <IconListDetails size={29} />
                    </div>
                    <h2>
                      {query || filter !== "all"
                        ? "No matching tasks"
                        : "Make room for your next idea"}
                    </h2>
                    <p>
                      {query || filter !== "all"
                        ? "Try another search or task filter."
                        : node
                          ? "Describe a task above. Your agent will handle it on the selected runtime."
                          : "Start by enrolling a runtime, then send your first task."}
                    </p>
                    {!node && (
                      <button
                        className="secondary"
                        onClick={() => setSurface("fleet")}
                      >
                        <IconServer size={17} />
                        Set up a runtime
                        <IconArrowRight size={16} />
                      </button>
                    )}
                  </div>
                ) : (
                  <div className="task-table">
                    <div className="table-labels">
                      <span>TASK</span>
                      <span>RUNTIME</span>
                      <span>STATUS</span>
                      <span>CREATED</span>
                      <span />
                    </div>
                    {shown.map((task) => (
                      <button
                        key={task.task_id}
                        className="task-row"
                        onClick={() => setSelected(task)}
                      >
                        <div className="task-title">
                          <strong>{taskTitle(task)}</strong>
                          <span>
                            {task.request.role} · {task.request.mode}
                          </span>
                        </div>
                        <span className="runtime-name">
                          <IconServer size={15} />
                          {nodes.find(
                            (fleet) => fleet.node.node_id === task.node_id,
                          )?.node.label ?? task.node_id.slice(0, 8)}
                        </span>
                        <span className={`status status-${taskStatus(task)}`}>
                          {statusLabel(taskStatus(task))}
                        </span>
                        <time>
                          {task.snapshot
                            ? new Date(
                                task.snapshot.run.created_at,
                              ).toLocaleDateString(undefined, {
                                month: "short",
                                day: "numeric",
                              })
                            : "Just queued"}
                        </time>
                        <IconArrowRight size={16} />
                      </button>
                    ))}
                  </div>
                )}
                {tasks.length === taskPages * 100 && (
                  <button
                    className="secondary"
                    onClick={() => setTaskPages((pages) => pages + 1)}
                  >
                    Load more tasks
                  </button>
                )}
              </section>
            </>
          )}
        </main>
        <footer className="workspace-footer">
          <span>
            <IconShieldLock size={13} /> Colossus runtime authority
          </span>
          <span>Cloud control plane · v1alpha1</span>
        </footer>
      </div>
    </div>
  );
}
