import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { ControlPlaneFrame, Button, DropdownSelect } from "@colossus/ui";
import colossusMark from "@colossus/ui/assets/colossus-mark.svg";
import {
  IconHome,
  IconServer,
  IconFolder,
  IconUsers,
  IconSettings,
  IconLogout,
  IconRefresh,
  IconLoader2,
  IconPlus,
  IconX,
} from "@tabler/icons-react";
import {
  AppearanceSettings,
  SendShortcutContext,
  useAppearance,
} from "./Appearance";
import {
  ApiFailure,
  request,
  projectPath,
  taskStatus,
  terminalStatuses,
  type FleetNode,
  type Host,
  type Task,
  type Thread,
  type ThreadDetailResponse,
} from "./api";
import {
  projectPermissions,
  type Me,
  type AuthConfig,
  type PublicSettings,
} from "./control-api";
import { Fleet } from "./Fleet";
import { TaskDetail } from "./TaskDetail";
import { TaskTable } from "./TaskTable";
import { RunComposer, type RunRequest } from "./RunComposer";
import { Home, LoadState } from "./Home";
import { Projects } from "./Projects";
import { AgentSidebar, AgentWorkspace } from "./AgentScope";
import { SignIn } from "./SignIn";
import { DocumentationLink } from "./DocumentationLink";
import { useResource } from "./resources";
const ThreadDetail = lazy(() =>
  import("./ThreadDetail").then((module) => ({ default: module.ThreadDetail })),
);
const Administration = lazy(() =>
  import("./Administration").then((module) => ({
    default: module.Administration,
  })),
);
import {
  NavigationProvider,
  useNavigation,
  restoreSignInReturn,
  RouteLink,
} from "./navigation";
import {
  agentHref,
  globalHref,
  projectHref,
  routeSurface,
  taskHref,
  threadHref,
  type Surface,
} from "./routes";
export function App() {
  const [me, setMe] = useState<Me | null>(null),
    [auth, setAuth] = useState<"loading" | "signed_out" | "ready">("loading"),
    [authConfig, setAuthConfig] = useState<AuthConfig | null>(null),
    [settings, setSettings] = useState<PublicSettings | null>(null),
    [error, setError] = useState("");
  const epoch = useRef(0);
  const loadMe = useCallback(async (signal?: AbortSignal) => {
    const generation = ++epoch.current;
    try {
      const response = await request<Me>("/api/me", undefined, signal);
      if (signal?.aborted || generation !== epoch.current) return;
      setMe(response);
      setAuth("ready");
      setError("");
      restoreSignInReturn();
    } catch (e) {
      if (!signal?.aborted && generation === epoch.current) {
        setMe(null);
        setAuth("signed_out");
        if (!(e instanceof ApiFailure && e.status === 403))
          setError(e instanceof Error ? e.message : "Sign-in unavailable.");
      }
    }
  }, []);
  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
    const reconcile = () => {
      if (pending) return;
      pending = true;
      void loadMe(abort.signal).finally(() => {
        pending = false;
      });
    };
    window.addEventListener("colossus:web:reconcile-identity", reconcile);
    return () => {
      abort.abort();
      window.removeEventListener("colossus:web:reconcile-identity", reconcile);
    };
  }, [loadMe]);
  useEffect(() => {
    const abort = new AbortController();
    void loadMe(abort.signal);
    void request<AuthConfig>("/api/auth/config", undefined, abort.signal)
      .then((value) => {
        if (!abort.signal.aborted) setAuthConfig(value);
      })
      .catch(() => {
        if (!abort.signal.aborted)
          setError(
            "Sign-in configuration could not be loaded. Refresh to retry.",
          );
      });
    void request<PublicSettings>("/api/settings", undefined, abort.signal)
      .then((value) => {
        if (!abort.signal.aborted) setSettings(value);
      })
      .catch(() => {});
    return () => abort.abort();
  }, [loadMe]);
  async function signOut() {
    try {
      await request("/auth/logout", {});
      epoch.current++;
      setMe(null);
      setAuth("signed_out");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Sign-out failed.");
    }
  }
  if (auth === "loading")
    return (
      <main className="startup" aria-busy="true">
        <img src={colossusMark} alt="Colossus" />
        <IconLoader2 size={20} className="spin" aria-hidden="true" />
        <p>Connecting to your Control Plane…</p>
      </main>
    );
  if (auth === "signed_out")
    return (
      <SignIn
        config={authConfig}
        classification={settings?.classification}
        error={error}
        onSignedIn={() => void loadMe()}
      />
    );
  if (!me) return null;
  return (
    <NavigationProvider>
      <AuthenticatedApp
        key={me.user.id}
        me={me}
        authConfig={authConfig}
        settings={settings}
        onSettings={setSettings}
        loadMe={() => void loadMe()}
        onSignOut={() => void signOut()}
      />
    </NavigationProvider>
  );
}

function AuthenticatedApp({
  me,
  authConfig,
  settings,
  onSettings,
  loadMe,
  onSignOut,
}: {
  me: Me;
  authConfig: AuthConfig | null;
  settings: PublicSettings | null;
  onSettings: (value: PublicSettings) => void;
  loadMe: () => void;
  onSignOut: () => void;
}) {
  const navigation = useNavigation(),
    { route } = navigation;
  const defaultProject =
    me.projects.find((item) => !item.archived)?.id ?? me.projects[0]?.id ?? "";
  const [preferredProject, setPreferredProject] = useState(defaultProject);
  const requestedProject = route.kind !== "invalid" ? route.project : undefined;
  const project =
    requestedProject ??
    (me.projects.some((item) => item.id === preferredProject)
      ? preferredProject
      : defaultProject);
  const authorizedProject =
    route.kind !== "invalid" && me.projects.some((item) => item.id === project);
  useEffect(() => {
    if (authorizedProject) setPreferredProject(project);
  }, [authorizedProject, project]);
  const surface = routeSurface(route);
  const { appearance, setAppearance } = useAppearance();
  const [nodeCache, setNodes] = useState<FleetNode[]>([]),
    [hostCache, setHosts] = useState<Host[]>([]),
    [taskCache, setTasks] = useState<Task[]>([]),
    [loadedProject, setLoadedProject] = useState(""),
    [nodePages, setNodePages] = useState(1),
    [hostPages, setHostPages] = useState(1),
    [taskPages, setTaskPages] = useState(1),
    [loading, setLoading] = useState(true),
    [error, setError] = useState(""),
    [deniedProject, setDeniedProject] = useState(""),
    [node, setNode] = useState(""),
    [busy, setBusy] = useState(false),
    [taskComposer, setTaskComposer] = useState(false);
  const currentProject = useRef(project),
    alive = useRef(true),
    inventoryEpoch = useRef(0);
  currentProject.current = project;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const nodes = loadedProject === project ? nodeCache : [],
    hosts = loadedProject === project ? hostCache : [],
    tasks = loadedProject === project ? taskCache : [];
  const permissions = authorizedProject ? projectPermissions(me, project) : [];
  const threadResource = useResource<ThreadDetailResponse>(
    route.kind === "thread" && authorizedProject
      ? `${projectPath(project)}/threads/${encodeURIComponent(route.thread)}`
      : null,
    0,
    me.user.id,
  );
  const taskResource = useResource<{ task: Task }>(
    route.kind === "task" && authorizedProject
      ? `${projectPath(project)}/tasks/${encodeURIComponent(route.task)}`
      : null,
    0,
    me.user.id,
  );
  const selectedThread =
    route.kind === "thread" &&
    !threadResource.error &&
    threadResource.data?.thread.project_id === project &&
    threadResource.data.thread.thread_id === route.thread
      ? threadResource.data.thread
      : null;
  const selectedTask =
    route.kind === "task" &&
    !taskResource.error &&
    taskResource.data?.task.project_id === project &&
    taskResource.data.task.task_id === route.task
      ? taskResource.data.task
      : null;
  const agentId =
    route.kind === "agent" ? route.node : (selectedThread?.node_id ?? "");
  const selectedAgent = useResource<FleetNode>(
    authorizedProject &&
      agentId &&
      !nodes.some((item) => item.node.node_id === agentId)
      ? `${projectPath(project)}/nodes/${encodeURIComponent(agentId)}`
      : null,
    3000,
    me.user.id,
  );
  const resolvedAgent =
    !selectedAgent.error &&
    selectedAgent.data?.node.project_id === project &&
    selectedAgent.data.node.node_id === agentId
      ? selectedAgent.data
      : null;
  const scopedNodes =
    resolvedAgent &&
    !nodes.some((item) => item.node.node_id === resolvedAgent.node.node_id)
      ? [...nodes, resolvedAgent]
      : nodes;
  const agent = scopedNodes.find((item) => item.node.node_id === agentId);
  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      if (!project || !authorizedProject) return;
      const generation = ++inventoryEpoch.current;
      async function pages<T>(
        kind: "tasks" | "nodes" | "hosts",
        count: number,
        id: (item: T) => string,
      ) {
        const items: T[] = [];
        let after = "";
        for (let page = 0; page < count; page++) {
          const response = await request<
              Record<string, T[]> & { next_cursor?: string | null }
            >(
              `${projectPath(project)}/${kind}?limit=100${after ? `&after=${encodeURIComponent(after)}` : ""}`,
              undefined,
              signal,
            ),
            batch = response[kind] ?? [];
          items.push(...batch);
          if (batch.length < 100) break;
          after = response.next_cursor ?? id(batch.at(-1)!);
        }
        return items;
      }
      try {
        const [fleet, work, inventory] = await Promise.all([
          pages<FleetNode>("nodes", nodePages, (item) => item.node.node_id),
          pages<Task>("tasks", taskPages, (item) => item.task_id),
          pages<Host>("hosts", hostPages, (item) => item.host_id),
        ]);
        if (
          signal?.aborted ||
          !alive.current ||
          currentProject.current !== project ||
          generation !== inventoryEpoch.current
        )
          return;
        setNodes(fleet);
        setTasks(work);
        setHosts(inventory);
        setLoadedProject(project);
        setLoading(false);
        setError("");
        setDeniedProject("");
      } catch (e) {
        if (
          !signal?.aborted &&
          alive.current &&
          currentProject.current === project &&
          generation === inventoryEpoch.current
        ) {
          setLoading(false);
          if (e instanceof ApiFailure && (e.status === 401 || e.status === 403))
            setDeniedProject(project);
          setError(
            e instanceof Error ? e.message : "Control Plane unavailable.",
          );
        }
      }
    },
    [project, authorizedProject, nodePages, taskPages, hostPages],
  );
  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
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
  useEffect(() => {
    setNodes([]);
    setHosts([]);
    setTasks([]);
    setLoadedProject("");
    setNodePages(1);
    setHostPages(1);
    setTaskPages(1);
    setLoading(true);
    setNode("");
    setBusy(false);
    setTaskComposer(false);
    setError("");
    setDeniedProject("");
  }, [project]);
  useEffect(() => {
    if (
      !node ||
      !nodes.some((item) => item.node.node_id === node && !item.node.revoked)
    )
      setNode(
        (
          nodes.find((item) => !item.node.revoked && item.presence?.ready) ??
          nodes.find((item) => !item.node.revoked)
        )?.node.node_id ?? "",
      );
  }, [nodes, node]);
  function openThread(thread: Thread) {
    navigation.go(threadHref(thread.project_id, thread.thread_id));
  }
  function openTask(task: Task) {
    navigation.go(
      task.thread_id
        ? threadHref(task.project_id, task.thread_id)
        : taskHref(task.project_id, task.task_id),
    );
  }
  function openAgent(id: string, view: "overview" | "policy" = "overview") {
    navigation.go(agentHref(project, id, view));
  }
  function selectProject(id: string) {
    setPreferredProject(id);
    navigation.go(
      route.kind === "project"
        ? projectHref(id, route.view)
        : surface === "projects"
          ? projectHref(id)
          : route.kind === "global"
            ? globalHref(surface, id, route.view)
            : globalHref("fleet", id),
    );
  }
  async function createTask(runRequest: RunRequest) {
    setBusy(true);
    setError("");
    const originalProject = project;
    try {
      const response = await request<{ task: Task }>(
        `${projectPath(project)}/tasks`,
        { node_id: node, request: runRequest },
      );
      if (!alive.current || currentProject.current !== originalProject)
        return false;
      openTask(response.task);
      void refresh();
      return true;
    } catch (e) {
      if (alive.current && currentProject.current === originalProject)
        setError(
          e instanceof Error
            ? e.message
            : "Task submission failed. Retry to reconcile the same task.",
        );
      return false;
    } finally {
      if (alive.current && currentProject.current === originalProject)
        setBusy(false);
    }
  }
  async function createThread(runRequest: RunRequest) {
    setBusy(true);
    setError("");
    const originalProject = project;
    try {
      const response = await request<{ thread: Thread }>(
        `${projectPath(project)}/threads`,
        { node_id: agentId, request: runRequest },
      );
      if (!alive.current || currentProject.current !== originalProject)
        return false;
      openThread(response.thread);
      void refresh();
      return true;
    } catch (e) {
      if (alive.current && currentProject.current === originalProject)
        setError(
          e instanceof Error
            ? e.message
            : "Conversation submission failed. Retry to reconcile the same request.",
        );
      return false;
    } finally {
      if (alive.current && currentProject.current === originalProject)
        setBusy(false);
    }
  }
  const active = tasks.filter(
      (task) => !terminalStatuses.has(taskStatus(task)),
    ).length,
    live = nodes.filter(
      (item) => item.presence?.ready && !item.node.revoked,
    ).length;
  const taskView = (
    <section
      className="managed-settings-body catalog-settings tasks-settings"
      aria-labelledby="tasks-heading"
    >
      <header className="catalog-heading">
        <div>
          <h2 id="tasks-heading">Tasks</h2>
          <p>
            Execution requests across this project’s agents. Open a task to view
            its conversation.
          </p>
        </div>
        <Button
          variant="primary"
          onClick={() => setTaskComposer((value) => !value)}
          disabled={!permissions.includes("execute") || !node}
        >
          <IconPlus size={16} aria-hidden="true" />
          {taskComposer ? "Close composer" : "New task"}
        </Button>
      </header>
      <p className="catalog-summary">
        {tasks.length} loaded tasks · {active} active
      </p>
      {taskComposer ? (
        <RunComposer
          onPolicy={() => openAgent(node, "policy")}
          nodes={scopedNodes}
          nodeId={node}
          onNodeChange={setNode}
          disabled={!permissions.includes("execute")}
          busy={busy}
          label="Create an agent task"
          action="Start task"
          onSubmit={createTask}
        />
      ) : null}
      <TaskTable
        key={project}
        project={project}
        tasks={tasks}
        nodes={scopedNodes}
        loading={loading}
        hasMore={tasks.length === taskPages * 100}
        onMore={() => setTaskPages((value) => value + 1)}
        onOpen={openTask}
        onFleet={() => navigation.go(globalHref("fleet", project))}
      />
    </section>
  );
  let unavailable = "";
  if (route.kind === "invalid") unavailable = route.message;
  else if (requestedProject && !authorizedProject)
    unavailable =
      "This project is unavailable or your signed-in identity does not have access. Choose an authorized project to continue.";
  else if (
    deniedProject === project &&
    surface !== "home" &&
    surface !== "admin" &&
    surface !== "settings"
  )
    unavailable =
      "Your project access is unavailable. Refresh your identity or select an authorized project.";
  else if (surface === "admin" && !me.user.is_admin)
    unavailable =
      "Your signed-in identity does not have Control Plane administration access.";
  else if (route.kind === "thread" && threadResource.error)
    unavailable = threadResource.error;
  else if (
    route.kind === "thread" &&
    !threadResource.loading &&
    !selectedThread
  )
    unavailable =
      "The requested conversation could not be found in this project.";
  else if (route.kind === "task" && taskResource.error)
    unavailable = taskResource.error;
  else if (route.kind === "task" && !taskResource.loading && !selectedTask)
    unavailable = "The requested task could not be found in this project.";
  else if (route.kind === "agent" && selectedAgent.error)
    unavailable = selectedAgent.error;
  else if (
    route.kind === "agent" &&
    !selectedAgent.loading &&
    !loading &&
    !agent
  )
    unavailable = "The requested agent could not be found in this project.";
  return (
    <SendShortcutContext value={appearance.sendShortcut}>
      <ControlPlaneFrame
        current={surface}
        navigation={[
          {
            id: "home",
            label: "Home",
            href: globalHref("home"),
            icon: <IconHome size={21} aria-hidden="true" />,
          },
          {
            id: "fleet",
            label: "Fleet",
            href: globalHref("fleet", authorizedProject ? project : undefined),
            icon: <IconServer size={21} aria-hidden="true" />,
          },
          {
            id: "projects",
            label: "Projects",
            href: globalHref("projects"),
            icon: <IconFolder size={21} aria-hidden="true" />,
          },
          ...(me.user.is_admin
            ? [
                {
                  id: "admin",
                  label: "Administration",
                  shortLabel: "Admin",
                  href: globalHref("admin"),
                  icon: <IconUsers size={21} aria-hidden="true" />,
                },
              ]
            : []),
          {
            id: "settings",
            label: "Settings",
            href: globalHref("settings"),
            icon: <IconSettings size={21} aria-hidden="true" />,
          },
        ]}
        onNavigate={(id) =>
          navigation.go(
            globalHref(
              id as Surface,
              id === "fleet" && authorizedProject ? project : undefined,
            ),
          )
        }
        classification={settings?.classification}
        user={
          <span className="rail-user" title={me.user.display_name}>
            {me.user.display_name.slice(0, 2).toUpperCase()}
          </span>
        }
        footer={
          <>
            <DocumentationLink />
            <button
              type="button"
              className="ui-icon-button"
              aria-label="Sign out"
              title="Sign out"
              onClick={onSignOut}
            >
              <IconLogout size={19} aria-hidden="true" />
            </button>
          </>
        }
        sidebar={
          !unavailable && agentId ? (
            <AgentSidebar
              key={`${me.user.id}:${project}:${agentId}`}
              project={project}
              nodes={scopedNodes}
              hosts={hosts}
              agentId={agentId}
              selected={selectedThread?.thread_id ?? ""}
              tasks={tasks}
              newDisabled={
                !permissions.includes("execute") || Boolean(agent?.node.revoked)
              }
              onAgent={(id) => openAgent(id)}
              onOpen={openThread}
              onNew={() =>
                navigation.go(agentHref(project, agentId, "threads", true))
              }
              onBack={() => navigation.back(globalHref("fleet", project))}
            />
          ) : undefined
        }
        header={
          <>
            <label className="header-project">
              <span className="sr-only">Project scope</span>
              <DropdownSelect
                aria-label="Select project"
                value={authorizedProject ? project : ""}
                onChange={(event) => selectProject(event.target.value)}
              >
                {!authorizedProject ? (
                  <option value="">Select an authorized project</option>
                ) : null}
                {me.projects.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                    {item.archived ? " · Archived" : ""}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            <span className="connection-indicator">
              <span className={`dot ${live ? "live" : ""}`} />
              {live} online
            </span>
            <button
              type="button"
              className="ui-icon-button"
              aria-label="Refresh project"
              onClick={() => {
                void refresh();
                loadMe();
                threadResource.refresh();
                taskResource.refresh();
                selectedAgent.refresh();
              }}
            >
              <IconRefresh size={16} aria-hidden="true" />
            </button>
          </>
        }
      >
        {error && !unavailable ? (
          <div role="alert" className="alert project-alert">
            {error}
            <button
              type="button"
              className="ui-icon-button"
              aria-label="Dismiss error"
              onClick={() => setError("")}
            >
              <IconX size={16} aria-hidden="true" />
            </button>
          </div>
        ) : null}
        <Suspense
          key={me.user.id}
          fallback={
            <p className="control-page muted" role="status">
              Loading view…
            </p>
          }
        >
          {unavailable ? (
            <section className="control-page">
              <header className="page-heading">
                <div>
                  <h1>Page unavailable</h1>
                  <p role="alert">{unavailable}</p>
                </div>
              </header>
              <RouteLink
                className="ui-button ui-button--secondary"
                href={globalHref("home")}
              >
                Go Home
              </RouteLink>
            </section>
          ) : route.kind === "thread" ? (
            selectedThread ? (
              <ThreadDetail
                key={`${project}:${selectedThread.thread_id}`}
                initial={selectedThread}
                project={project}
                nodes={scopedNodes}
                permissions={permissions}
                onBack={() =>
                  navigation.back(agentHref(project, selectedThread.node_id))
                }
                backHref={agentHref(project, selectedThread.node_id)}
                backLabel="Back"
                onChanged={() => void refresh()}
                onPolicy={() => openAgent(selectedThread.node_id, "policy")}
              />
            ) : (
              <LoadState {...threadResource} retry={threadResource.refresh} />
            )
          ) : route.kind === "task" ? (
            selectedTask ? (
              <TaskDetail
                key={`${project}:${selectedTask.task_id}`}
                initial={selectedTask}
                project={project}
                permissions={selectedTask.source_read_only ? [] : permissions}
                nodeLabel={
                  nodes.find(
                    (item) => item.node.node_id === selectedTask.node_id,
                  )?.node.label ?? selectedTask.node_id
                }
                onBack={() => navigation.back(projectHref(project, "tasks"))}
                backHref={projectHref(project, "tasks")}
              />
            ) : (
              <LoadState {...taskResource} retry={taskResource.refresh} />
            )
          ) : surface === "home" ? (
            <Home
              onOpen={openThread}
              onFleet={() =>
                navigation.go(globalHref("fleet", project || undefined))
              }
            />
          ) : surface === "settings" ? (
            <div className="control-page">
              <header className="page-heading">
                <div>
                  <span className="eyebrow">Settings</span>
                  <h1>Personal preferences</h1>
                  <p>Customize the Control Plane for this browser.</p>
                </div>
              </header>
              <AppearanceSettings
                appearance={appearance}
                onChange={setAppearance}
              />
              <section className="control-card">
                <header>
                  <h3>Signed-in identity</h3>
                </header>
                <p>{me.user.display_name}</p>
                <p className="field-help">
                  {me.user.email ?? ""}
                  {me.user.is_admin ? " · Control Plane administrator" : ""}
                </p>
              </section>
            </div>
          ) : surface === "admin" ? (
            <Administration
              authConfig={authConfig}
              onRefresh={loadMe}
              onSettings={onSettings}
              view={route.kind === "global" ? (route.view ?? "users") : "users"}
              onView={(view) =>
                navigation.go(globalHref("admin", undefined, view))
              }
            />
          ) : surface === "projects" ? (
            <Projects
              key={project}
              me={me}
              selected={route.kind === "project" ? project : ""}
              onSelect={(id) => navigation.go(projectHref(id))}
              permissions={permissions}
              onRefresh={loadMe}
              tasks={taskView}
              view={route.kind === "project" ? route.view : "overview"}
              onView={(view) => navigation.go(projectHref(project, view))}
            />
          ) : !project ? (
            <div className="empty-state">
              <h2>No project access</h2>
              <p>Ask your administrator for membership in a project.</p>
            </div>
          ) : route.kind === "agent" ? (
            <AgentWorkspace
              key={agentId}
              project={project}
              agent={agent}
              nodes={scopedNodes}
              permissions={permissions}
              creating={route.compose}
              busy={busy}
              onCreate={createThread}
              onNew={() =>
                navigation.go(
                  agentHref(project, agentId, route.view, !route.compose),
                )
              }
              onOpen={openThread}
              view={route.view}
              onView={(view) =>
                navigation.go(agentHref(project, agentId, view))
              }
            />
          ) : (
            <Fleet
              key={project}
              hosts={hosts}
              nodes={nodes}
              project={project}
              permissions={permissions}
              onRefresh={() => void refresh()}
              onError={setError}
              hasMore={nodes.length === nodePages * 100}
              hasMoreHosts={hosts.length === hostPages * 100}
              onMoreHosts={() => setHostPages((value) => value + 1)}
              onMore={() => setNodePages((value) => value + 1)}
              onOpenAgent={(id) => openAgent(id)}
            />
          )}
        </Suspense>
      </ControlPlaneFrame>
    </SendShortcutContext>
  );
}
