import type { ForkDraftSource } from "./ThreadBranchDraft";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  ControlPlaneFrame,
  ControlPlaneProjectSwitcher,
  ControlPlaneAccountMenu,
  Button,
} from "@colossus/ui";

import { IconRefresh, IconPlus, IconX } from "@tabler/icons-react";
import { SendShortcutContext, useAppearance } from "./Appearance";
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
import { Projects, CreateProject } from "./Projects";
import { AgentSidebar, AgentWorkspace } from "./AgentScope";
import { HostSidebar, HostOverview, useHostRoster } from "./HostScope";
import { workspaceName } from "./workspace-navigation";

import { DocumentationLink } from "./DocumentationLink";
import { ChatHistorySidebar } from "./ChatHistorySidebar";
import { WebSettings } from "./WebSettings";
import { useResource } from "./resources";
import { useInstanceAnnouncements } from "./InstanceAnnouncements";
import { controlNavigation } from "./control-navigation";
const ThreadDetail = lazy(() =>
  import("./ThreadDetail").then((module) => ({ default: module.ThreadDetail })),
);
const Chat = lazy(() =>
  import("./Chat").then((module) => ({ default: module.Chat })),
);
const Hosted = lazy(() =>
  import("./Hosted").then((module) => ({ default: module.Hosted })),
);
const Administration = lazy(() =>
  import("./Administration").then((module) => ({
    default: module.Administration,
  })),
);
import { useNavigation, RouteLink } from "./navigation";
import { conversationHref, isPersonalHostedChat } from "./conversation-route";
import {
  agentHref,
  chatConnectionsHref,
  chatThreadHref,
  globalHref,
  hostHref,
  projectHref,
  projectSettingsHref,
  personalSettingsHref,
  routeSurface,
  taskHref,
  threadHref,
} from "./routes";
export function AuthenticatedApp({
  me,
  authConfig,
  settings,
  onSettings,
  loadMe,
  onSignOut,
  signOutError = "",
  signingOut = false,
}: {
  me: Me;
  authConfig: AuthConfig | null;
  settings: PublicSettings | null;
  onSettings: (value: PublicSettings) => void;
  loadMe: () => void;
  onSignOut: () => void;
  signOutError?: string;
  signingOut?: boolean;
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
  const [chatRefresh, setChatRefresh] = useState(0);
  const [forkDraft, setForkDraft] = useState<ForkDraftSource | null>(null);
  const returnToWork = useRef(globalHref("chat", project));
  const visitedChat = useRef("");
  const lastChat = useRef<{
    project: string;
    href: string;
    thread: Thread | null;
  }>({ project, href: globalHref("chat", project), thread: null });
  const { appearance, setAppearance } = useAppearance();
  const announcements = useInstanceAnnouncements(me.user.id);
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
  const appNavigation = controlNavigation(me, route, project);
  const threadResource = useResource<ThreadDetailResponse>(
    (route.kind === "thread" || route.kind === "chat") && authorizedProject
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
    (route.kind === "thread" || route.kind === "chat") &&
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
  const redirectToChat =
    route.kind === "thread" &&
    selectedThread?.owner_user_id === me.user.id &&
    isPersonalHostedChat(selectedThread)
      ? conversationHref(selectedThread)
      : null;
  useEffect(() => {
    if (redirectToChat) navigation.go(redirectToChat, true);
  }, [redirectToChat, navigation.go]);
  if (surface === "chat") visitedChat.current = project;
  if (
    surface === "chat" &&
    route.kind !== "chat-connections" &&
    (route.kind !== "chat" || selectedThread)
  ) {
    lastChat.current = {
      project,
      href: navigation.href,
      thread: selectedThread,
    };
  }
  const personalSelection =
    selectedThread ??
    (lastChat.current.project === project &&
    (route.kind === "chat-connections" ||
      surface === "settings" ||
      surface === "hosted" ||
      (route.kind === "chat" && lastChat.current.href === navigation.href))
      ? lastChat.current.thread
      : null);
  function openConnections() {
    navigation.go(chatConnectionsHref(project));
  }
  function openProjectSettings() {
    returnToWork.current = navigation.href;
    navigation.go(projectSettingsHref(project));
  }
  const [workspaceSessions, setWorkspaceSessions] = useState<
    Record<string, string>
  >({});
  useEffect(() => {
    if (selectedThread)
      setWorkspaceSessions((current) => ({
        ...current,
        [project + ":" + selectedThread.node_id]: selectedThread.thread_id,
      }));
  }, [project, selectedThread?.node_id, selectedThread?.thread_id]);
  const agentId =
    route.kind === "agent"
      ? route.node
      : (selectedThread?.node_id ?? selectedTask?.node_id ?? "");
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
  const projectNodes =
    resolvedAgent &&
    !nodes.some((item) => item.node.node_id === resolvedAgent.node.node_id)
      ? [...nodes, resolvedAgent]
      : nodes;
  const agent = projectNodes.find((item) => item.node.node_id === agentId);
  const hostId =
    route.kind === "host"
      ? route.host
      : (agent?.node.host_id ?? selectedThread?.host_id ?? "");
  const hostDetail = useResource<{ host: Host }>(
    authorizedProject && hostId
      ? `${projectPath(project)}/hosts/${encodeURIComponent(hostId)}`
      : null,
    5000,
    me.user.id,
  );
  const host =
    hostDetail.data?.host.project_id === project &&
    hostDetail.data.host.host_id === hostId
      ? hostDetail.data.host
      : hosts.find((item) => item.host_id === hostId);
  const roster = useHostRoster(
    authorizedProject ? project : "",
    authorizedProject ? hostId : "",
    me.user.id,
  );
  const scopedNodes = [
    ...new Map(
      [...projectNodes, ...roster.nodes].map((item) => [
        item.node.node_id,
        item,
      ]),
    ).values(),
  ];
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
        const [fleet, work, inventory] = await Promise.allSettled([
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
        if (fleet.status === "fulfilled") setNodes(fleet.value);
        if (work.status === "fulfilled") setTasks(work.value);
        if (inventory.status === "fulfilled") setHosts(inventory.value);
        setLoadedProject(project);
        setLoading(false);
        const failures = [fleet, work, inventory].filter(
          (result) => result.status === "rejected",
        );
        const denied = failures.find(
          (result) =>
            result.status === "rejected" &&
            result.reason instanceof ApiFailure &&
            [401, 403].includes(result.reason.status),
        );
        setDeniedProject(denied ? project : "");
        setError(
          failures.length
            ? "Some project data could not be refreshed. Available hosts and workspaces are shown."
            : "",
        );
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
    navigation.go(conversationHref(thread));
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
  async function revokeWorkspace() {
    if (!agent || busy || !permissions.includes("administer")) return;
    if (
      !confirm(
        `Revoke ${workspaceName(agent)}? Its Control Plane connection will close. Accepted work continues locally.`,
      )
    )
      return;
    setBusy(true);
    setError("");
    try {
      await request(
        `${projectPath(project)}/nodes/${encodeURIComponent(agentId)}/revoke`,
        { revision: agent.node.revision },
      );
      roster.refresh();
      void refresh();
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "The connection could not be revoked.",
      );
    } finally {
      setBusy(false);
    }
  }
  function selectProject(id: string) {
    setPreferredProject(id);
    navigation.go(
      route.kind === "project"
        ? projectHref(id, route.view)
        : route.kind === "project-settings"
          ? projectSettingsHref(id, route.section)
          : surface === "projects"
            ? projectHref(id)
            : route.kind === "global"
              ? surface === "settings"
                ? personalSettingsHref(route.section, id)
                : globalHref(surface, id, route.view)
              : globalHref(
                  surface === "chat" || surface === "hosted"
                    ? surface
                    : "fleet",
                  id,
                ),
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
  else if (
    route.kind === "project-settings" &&
    !permissions.includes("administer")
  )
    unavailable =
      "Your signed-in identity does not have administration access to this project.";
  else if (
    (route.kind === "thread" || route.kind === "chat") &&
    threadResource.error
  )
    unavailable = threadResource.error;
  else if (
    (route.kind === "thread" || route.kind === "chat") &&
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
  else if (route.kind === "host" && hostDetail.error)
    unavailable = hostDetail.error;
  return (
    <SendShortcutContext value={appearance.sendShortcut}>
      <ControlPlaneFrame
        activeScopeKey={JSON.stringify(route)}
        current={appNavigation.current}
        navigation={appNavigation.items}
        collapsibleNavigationGroups={[
          "Project Administration",
          "Control Plane Admin",
        ]}
        onNavigate={(id) => {
          const destination = appNavigation.items.find(
            (item) => item.id === id,
          )?.href;
          if (!destination) return;
          if (id === "settings" || id.startsWith("project-"))
            returnToWork.current = navigation.href;
          navigation.go(destination);
        }}
        title={
          <span className="control-window-context">
            <span>
              {surface === "admin"
                ? "Control Plane"
                : surface === "settings" && route.kind !== "project-settings"
                  ? "Personal"
                  : (me.projects.find((item) => item.id === project)?.name ??
                    "Colossus")}
            </span>
            <span aria-hidden="true">/</span>
            <span>
              {surface === "chat"
                ? route.kind === "chat-connections"
                  ? "Connections"
                  : "Chat"
                : surface === "settings"
                  ? route.kind === "project-settings"
                    ? (appNavigation.items.find(
                        (item) => item.id === appNavigation.current,
                      )?.label ?? "Project settings")
                    : "Settings"
                  : surface === "home"
                    ? "Home"
                    : surface === "hosted"
                      ? "Hosted"
                      : surface === "fleet"
                        ? "Fleet"
                        : surface === "projects"
                          ? "Projects"
                          : (appNavigation.items.find(
                              (item) => item.id === appNavigation.current,
                            )?.label ?? "Administration")}
            </span>
          </span>
        }
        classification={settings?.classification}
        announcement={announcements.notice}
        user={
          <ControlPlaneAccountMenu
            name={me.user.display_name}
            detail={me.user.email ?? "Signed in"}
            onSettings={() => {
              returnToWork.current = navigation.href;
              navigation.go(
                personalSettingsHref(
                  "appearance",
                  authorizedProject ? project : undefined,
                ),
              );
            }}
            onSignOut={onSignOut}
            signingOut={signingOut}
          />
        }
        footer={<DocumentationLink showLabel />}
        sidebar={
          surface === "chat" && project && authorizedProject ? (
            <ChatHistorySidebar
              key={`${me.user.id}:${project}`}
              project={project}
              identity={me.user.id}
              selected={personalSelection}
              refreshKey={chatRefresh}
              canControl={permissions.includes("control")}
              onThreadChanged={() => {
                threadResource.refresh();
                setChatRefresh((value) => value + 1);
              }}
              onFork={
                permissions.includes("read") && permissions.includes("execute")
                  ? (parent, source) => {
                      setForkDraft({
                        parent,
                        source,
                        key: crypto.randomUUID(),
                      });
                      navigation.go(chatThreadHref(project, parent.thread_id));
                    }
                  : undefined
              }
              onConnections={openConnections}
              onProjectSettings={
                permissions.includes("administer")
                  ? openProjectSettings
                  : undefined
              }
            />
          ) : !unavailable && host && hostId ? (
            <HostSidebar
              key={`${me.user.id}:${project}:${hostId}`}
              project={project}
              host={host}
              nodes={roster.nodes}
              identity={me.user.id}
              agentId={agentId}
              selected={
                selectedThread?.thread_id ??
                workspaceSessions[project + ":" + agentId] ??
                ""
              }
              canExecute={permissions.includes("execute")}
              onAgent={(id) => openAgent(id)}
              onOpen={openThread}
              onNew={(id) =>
                navigation.go(agentHref(project, id, "threads", true))
              }
              onBack={() => navigation.go(globalHref("fleet", project))}
              hasMore={roster.hasMore}
              onMore={() => void roster.more()}
              view={route.kind === "agent" ? route.view : "threads"}
              onView={(view) =>
                navigation.go(agentHref(project, agentId, view))
              }
              moreBusy={roster.busy}
              error={roster.error}
            />
          ) : !unavailable && agentId && !hostId ? (
            <AgentSidebar
              key={`${me.user.id}:${project}:${agentId}`}
              project={project}
              nodes={scopedNodes}
              hosts={hosts}
              agentId={agentId}
              selected={
                selectedThread?.thread_id ??
                workspaceSessions[project + ":" + agentId] ??
                ""
              }
              tasks={tasks}
              view={route.kind === "agent" ? route.view : "threads"}
              onView={(view) =>
                navigation.go(agentHref(project, agentId, view))
              }
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
        sidebarHeader={
          <ControlPlaneProjectSwitcher
            projects={me.projects.map((item) => ({
              id: item.id,
              name: item.name + (item.archived ? " · Archived" : ""),
            }))}
            value={authorizedProject ? project : ""}
            onValueChange={selectProject}
            onManageProjects={() =>
              navigation.go(
                globalHref("projects", authorizedProject ? project : undefined),
              )
            }
          />
        }
        header={
          <>
            {announcements.control}
            {surface !== "chat" ? (
              <span className="connection-indicator">
                <span className={`dot ${live ? "live" : ""}`} />
                {live} online
              </span>
            ) : null}
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
                announcements.refresh();
              }}
            >
              <IconRefresh size={16} aria-hidden="true" />
            </button>
          </>
        }
      >
        {signOutError ? (
          <div role="alert" className="alert project-alert">
            <span>{signOutError}</span>
            <Button disabled={signingOut} onClick={onSignOut}>
              Retry sign out
            </Button>
          </div>
        ) : null}
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
          {!unavailable &&
          authorizedProject &&
          project &&
          (surface === "chat" ||
            ((surface === "settings" || surface === "hosted") &&
              visitedChat.current === project)) ? (
            <div
              className="chat-preserved-workspace"
              hidden={surface !== "chat"}
            >
              <Chat
                key={`${me.user.id}:${project}`}
                project={project}
                identity={me.user.id}
                permissions={permissions}
                selected={personalSelection}
                forkDraft={
                  forkDraft?.parent.project_id === project &&
                  forkDraft.parent.owner_user_id === me.user.id
                    ? forkDraft
                    : null
                }
                onCloseFork={() => setForkDraft(null)}
                onFork={(parent, source) =>
                  setForkDraft({ parent, source, key: crypto.randomUUID() })
                }
                view={
                  route.kind === "chat-connections"
                    ? "connections"
                    : "conversation"
                }
                onConnections={openConnections}
                onProjectSettings={
                  permissions.includes("administer")
                    ? openProjectSettings
                    : undefined
                }
                onCloseConnections={() =>
                  navigation.go(
                    lastChat.current.project === project
                      ? lastChat.current.href
                      : globalHref("chat", project),
                  )
                }
                detailLoading={threadResource.loading}
                onChanged={() => {
                  threadResource.refresh();
                  setChatRefresh((value) => value + 1);
                }}
              />
            </div>
          ) : null}
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
          ) : surface === "chat" ? (
            project ? null : (
              <div className="empty-state">
                <h2>No project access</h2>
                <p>Choose an authorized project to start a chat.</p>
              </div>
            )
          ) : route.kind === "host" ? (
            host ? (
              <HostOverview
                host={host}
                nodes={roster.nodes}
                project={project}
                onAgent={(id) => openAgent(id)}
              />
            ) : (
              <LoadState {...hostDetail} retry={hostDetail.refresh} />
            )
          ) : route.kind === "thread" ? (
            selectedThread && !redirectToChat ? (
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
                nodeLabel={(() => {
                  const item = scopedNodes.find(
                    (item) => item.node.node_id === selectedTask.node_id,
                  );
                  return item ? workspaceName(item) : selectedTask.node_id;
                })()}
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
            <WebSettings
              key={`${me.user.id}:${project}`}
              project={project}
              projectName={
                me.projects.find((item) => item.id === project)?.name ?? ""
              }
              identity={me.user.id}
              displayName={me.user.display_name}
              email={me.user.email}
              canAdminister={permissions.includes("administer")}
              canConnect={
                permissions.includes("execute") &&
                permissions.includes("control")
              }
              appearance={appearance}
              onAppearance={setAppearance}
              isGlobalAdmin={me.user.is_admin}
              onEnvironmentSettings={onSettings}
              onInstanceSettings={() =>
                navigation.go(globalHref("admin", undefined, "providers"))
              }
              initialScope={
                route.kind === "project-settings" ? "space" : "global"
              }
              initialSection={
                route.kind === "project-settings"
                  ? route.section
                  : route.kind === "global"
                    ? route.section
                    : undefined
              }
              onSectionChange={(scope, section) =>
                navigation.go(
                  scope === "space"
                    ? projectSettingsHref(
                        project,
                        section as import("./routes").ConnectionSettingsSection,
                      )
                    : personalSettingsHref(
                        section as import("./routes").PersonalSettingsSection,
                        authorizedProject ? project : undefined,
                      ),
                )
              }
              onReturn={() => navigation.go(returnToWork.current)}
              onConnections={openConnections}
              onSaved={() => {
                threadResource.refresh();
                setChatRefresh((value) => value + 1);
              }}
            />
          ) : surface === "admin" ? (
            <Administration
              identity={me.user.id}
              projects={me.projects}
              selectedProject={project}
              authConfig={authConfig}
              onRefresh={loadMe}
              onSettings={onSettings}
              currentUserId={me.user.id}
              userId={route.kind === "global" ? route.user : undefined}
              userView={route.kind === "global" ? route.userView : undefined}
              userProject={route.kind === "global" ? route.project : undefined}
              view={route.kind === "global" ? (route.view ?? "users") : "users"}
              onView={(view) =>
                navigation.go(globalHref("admin", undefined, view))
              }
            />
          ) : route.kind === "project-create" ? (
            <CreateProject
              me={me}
              onCancel={() => navigation.go(globalHref("projects"))}
              onSaved={() => {
                loadMe();
                navigation.go(globalHref("projects"));
              }}
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
          ) : surface === "hosted" ? (
            <Hosted
              key={`${me.user.id}:${project}:${route.kind === "hosted-agent" ? route.node : "inventory"}`}
              project={project}
              identity={me.user.id}
              permissions={permissions}
              node={route.kind === "hosted-agent" ? route.node : undefined}
              chatHref={
                lastChat.current.project === project
                  ? lastChat.current.href
                  : globalHref("chat", project)
              }
              onProjectSettings={openProjectSettings}
            />
          ) : route.kind === "agent" ? (
            <AgentWorkspace
              key={`${me.user.id}:${project}:${agentId}`}
              project={project}
              agent={agent}
              nodes={scopedNodes}
              permissions={permissions}
              contextThread={workspaceSessions[project + ":" + agentId] ?? null}
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
              onRevoke={() => void revokeWorkspace()}
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
              onOpenHost={(id) => navigation.go(hostHref(project, id))}
            />
          )}
        </Suspense>
      </ControlPlaneFrame>
    </SendShortcutContext>
  );
}
