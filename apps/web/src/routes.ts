import type { SettingsEditorLocation } from "./settings-editor";
export type Surface =
  "home" | "chat" | "hosted" | "fleet" | "projects" | "admin" | "settings";
export type ProjectView =
  "overview" | "tasks" | "analytics" | "access" | "settings";
export type AgentView =
  | "overview"
  | "threads"
  | "analytics"
  | "policy"
  | "workflows"
  | "schedules"
  | "capabilities"
  | "plugins"
  | "library"
  | "connections";
export const connectionSettingsSections = [
  "providers",
  "models",
  "mcp",
  "routing",
  "search",
  "telemetry",
  "credentials",
  "variables",
] as const;
export type ConnectionSettingsSection =
  (typeof connectionSettingsSections)[number];
export type PersonalSettingsSection =
  "appearance" | "account" | "credentials" | "connections" | "variables";
export type AdminUserView = "overview" | "projects" | "workspaces" | "chats";
export type AdminView =
  | "users"
  | "projects"
  | "settings"
  | "announcements"
  | "appearance"
  | ConnectionSettingsSection;
export type Route =
  | {
      kind: "global";
      surface: Surface;
      project?: string;
      view?: AdminView;
      user?: string;
      userView?: AdminUserView;
      section?: PersonalSettingsSection;
      editor?: SettingsEditorLocation;
    }
  | { kind: "project-create"; project?: undefined }
  | { kind: "project"; project: string; view: ProjectView }
  | {
      kind: "project-settings";
      project: string;
      section: ConnectionSettingsSection;
      editor?: SettingsEditorLocation;
    }
  | { kind: "host"; project: string; host: string }
  | { kind: "hosted-agent"; project: string; node: string }
  | {
      kind: "agent";
      project: string;
      node: string;
      view: AgentView;
      compose: boolean;
    }
  | { kind: "thread"; project: string; thread: string }
  | { kind: "chat"; project: string; thread: string }
  | { kind: "chat-connections"; project: string }
  | { kind: "task"; project: string; task: string }
  | { kind: "invalid"; message: string };

const projectViews = new Set([
  "overview",
  "tasks",
  "analytics",
  "access",
  "settings",
]);
const agentViews = new Set([
  "overview",
  "threads",
  "analytics",
  "policy",
  "workflows",
  "schedules",
  "capabilities",
  "plugins",
  "library",
  "connections",
]);
const adminViews = new Set([
  "users",
  "projects",
  "settings",
  "announcements",
  "appearance",
  ...connectionSettingsSections,
]);
const adminUserViews = new Set(["overview", "projects", "workspaces", "chats"]);
const personalSettingsSections = new Set([
  "appearance",
  "account",
  "credentials",
  "connections",
  "variables",
]);
const invalid = (): Route => ({
  kind: "invalid",
  message:
    "This address does not identify a Control Plane page. Check the link or choose a page from the navigation.",
});
function validId(value: string) {
  return (
    value.length > 0 &&
    value.length <= 256 &&
    value !== "." &&
    value !== ".." &&
    !/[\s/\\\u0000-\u001f\u007f]/u.test(value)
  );
}
function id(value: string) {
  if (!validId(value)) throw new Error("Invalid resource identifier.");
  return encodeURIComponent(value);
}
export function globalHref(
  surface: Surface,
  project?: string,
  view?: AdminView,
) {
  const base =
    surface === "home"
      ? "/"
      : `/${surface}${surface === "admin" && view ? `/${view}` : ""}`;
  return project ? `${base}?project=${id(project)}` : base;
}
export const adminUserHref = (
  user: string,
  view: AdminUserView = "overview",
  project?: string,
) =>
  `/admin/users/${id(user)}${view === "overview" ? "" : `/${view}`}${project ? `?project=${id(project)}` : ""}`;
export const projectHref = (project: string, view: ProjectView = "overview") =>
  `/projects/${id(project)}/${view}`;
export const projectCreateHref = () => "/projects/new";
export const projectSettingsHref = (
  project: string,
  section: ConnectionSettingsSection = "providers",
) => `/projects/${id(project)}/settings/${section}`;
export const personalSettingsHref = (
  section: PersonalSettingsSection = "appearance",
  project?: string,
) => `/settings/${section}${project ? `?project=${id(project)}` : ""}`;
/** Editor URLs contain identifiers only. Credentials and drafts never enter history. */
export function settingsEditorHref(
  base: string,
  editor: SettingsEditorLocation,
) {
  const url = new URL(base, "https://control.invalid");
  if (editor.target) {
    if (editor.action !== "new" || !url.pathname.endsWith("/credentials"))
      throw new Error("Connection targets require a new credential form.");
    if (!validId(editor.target.id))
      throw new Error("Invalid connection identifier.");
    url.searchParams.set("kind", editor.target.kind);
    url.searchParams.set("connection", editor.target.id);
  }
  url.pathname = `${url.pathname.replace(/\/$/, "")}/${editor.action === "new" ? "new" : editor.id ? `${id(editor.id)}/edit` : "edit"}`;
  return `${url.pathname}${url.search}`;
}
function editorParts(
  parts: string[],
  singleton: boolean,
): SettingsEditorLocation | null {
  if (parts.length === 1 && parts[0] === "new" && !singleton)
    return { action: "new" };
  if (parts.length === 1 && parts[0] === "edit" && singleton)
    return { action: "edit" };
  if (parts.length === 2 && parts[1] === "edit" && !singleton)
    return { action: "edit", id: parts[0]! };
  return null;
}
export const hostHref = (project: string, host: string) =>
  `/projects/${id(project)}/hosts/${id(host)}`;
export const hostedAgentHref = (project: string, node: string) =>
  `/projects/${id(project)}/hosted/${id(node)}`;
export const agentHref = (
  project: string,
  node: string,
  view: AgentView = "overview",
  compose = false,
) =>
  `/projects/${id(project)}/agents/${id(node)}/${view}${compose ? "?compose=1" : ""}`;
export const threadHref = (project: string, thread: string) =>
  `/projects/${id(project)}/threads/${id(thread)}`;
export const chatThreadHref = (project: string, thread: string) =>
  `/projects/${id(project)}/chat/${id(thread)}`;
export const chatConnectionsHref = (project: string) =>
  `/projects/${id(project)}/chat/connections`;
export const taskHref = (project: string, task: string) =>
  `/projects/${id(project)}/tasks/${id(task)}`;

/** Paths select a view only. Every resource still requires an authorized API read. */
export function parseRoute(href: string, origin: string): Route {
  if (href.length > 2048) return invalid();
  try {
    const url = new URL(href, origin);
    if (url.origin !== origin || url.username || url.password) return invalid();
    const raw = url.pathname.split("/").slice(1);
    if (raw.at(-1) === "") raw.pop();
    const parts = raw.map(decodeURIComponent);
    if (parts.some((part) => !validId(part))) return invalid();
    const query = url.searchParams;
    if ([...query.keys()].some((key) => query.getAll(key).length !== 1))
      return invalid();
    const first = parts[0];
    const editorSection =
      first === "admin"
        ? parts[1]
        : first === "settings"
          ? parts[1]
          : first === "projects" && parts[2] === "settings"
            ? parts[3]
            : undefined;
    const editorTail = first === "projects" ? parts.slice(4) : parts.slice(2);
    const editable =
      first === "admin"
        ? ["users", "announcements", ...connectionSettingsSections]
        : first === "settings"
          ? ["credentials", "variables"]
          : connectionSettingsSections;
    if (
      editorSection &&
      (editable as readonly string[]).includes(editorSection) &&
      editorTail.length
    ) {
      const singleton = ["routing", "telemetry"].includes(editorSection);
      const editor: SettingsEditorLocation | null =
        editorSection === "search" &&
        editorTail.length === 1 &&
        editorTail[0] === "edit"
          ? { action: "edit" as const }
          : editorParts(editorTail, singleton);
      if (editor) {
        const kind = query.get("kind");
        const connection = query.get("connection");
        const hasTarget = query.has("kind") || query.has("connection");
        if (hasTarget) {
          if (first !== "projects" && !query.get("project")) return invalid();
          if (
            editorSection !== "credentials" ||
            editor.action !== "new" ||
            !kind ||
            !["provider", "mcp", "search"].includes(kind) ||
            !connection ||
            !validId(connection)
          )
            return invalid();
          editor.target = {
            kind: kind as "provider" | "mcp" | "search",
            id: connection,
          };
        }
        if (first === "projects")
          return [...query.keys()].some(
            (key) => key !== "kind" && key !== "connection",
          )
            ? invalid()
            : {
                kind: "project-settings",
                project: parts[1]!,
                section: editorSection as ConnectionSettingsSection,
                editor,
              };
        if (
          [...query.keys()].some(
            (key) => !["project", "kind", "connection"].includes(key),
          )
        )
          return invalid();
        const project = query.get("project");
        if (project !== null && !validId(project)) return invalid();
        return {
          kind: "global",
          surface: first === "admin" ? "admin" : "settings",
          ...(first === "admin"
            ? { view: editorSection as AdminView }
            : { section: editorSection as PersonalSettingsSection }),
          ...(project ? { project } : {}),
          editor,
        };
      }
      // User detail tabs are also recognized below; all other editor tails fail closed.
      if (!(first === "admin" && editorSection === "users")) return invalid();
    }
    if (
      first === "admin" &&
      parts[1] === "users" &&
      (parts.length === 3 ||
        (parts.length === 4 && adminUserViews.has(parts[3]!)))
    ) {
      if ([...query.keys()].some((key) => key !== "project")) return invalid();
      const project = query.get("project");
      if (project !== null && !validId(project)) return invalid();
      return {
        kind: "global",
        surface: "admin",
        view: "users",
        user: parts[2]!,
        userView: (parts[3] ?? "overview") as AdminUserView,
        ...(project ? { project } : {}),
      };
    }
    if (
      first === "settings" &&
      parts.length === 2 &&
      personalSettingsSections.has(parts[1]!)
    ) {
      if ([...query.keys()].some((key) => key !== "project")) return invalid();
      const project = query.get("project");
      if (project !== null && !validId(project)) return invalid();
      return {
        kind: "global",
        surface: "settings",
        section: parts[1] as PersonalSettingsSection,
        ...(project ? { project } : {}),
      };
    }
    if (
      !parts.length ||
      (parts.length === 1 &&
        ["chat", "hosted", "fleet", "projects", "admin", "settings"].includes(
          first!,
        )) ||
      (first === "admin" && parts.length === 2 && adminViews.has(parts[1]!))
    ) {
      if ([...query.keys()].some((key) => key !== "project")) return invalid();
      const project = query.get("project");
      if (project !== null && !validId(project)) return invalid();
      return {
        kind: "global",
        surface: (first ?? "home") as Surface,
        ...(project ? { project } : {}),
        ...(parts[1] ? { view: parts[1] as AdminView } : {}),
      };
    }
    if (first === "projects" && parts.length === 2 && parts[1] === "new") {
      return query.size ? invalid() : { kind: "project-create" };
    }
    if (first !== "projects" || !parts[1]) return invalid();
    const project = parts[1];
    if (
      parts.length === 4 &&
      parts[2] === "settings" &&
      (connectionSettingsSections as readonly string[]).includes(parts[3]!)
    ) {
      return query.size
        ? invalid()
        : {
            kind: "project-settings",
            project,
            section: parts[3] as ConnectionSettingsSection,
          };
    }
    if (parts.length === 3 && projectViews.has(parts[2]!)) {
      if (query.size) return invalid();
      return { kind: "project", project, view: parts[2] as ProjectView };
    }
    if (
      parts.length === 5 &&
      parts[2] === "agents" &&
      agentViews.has(parts[4]!)
    ) {
      if (
        [...query.keys()].some((key) => key !== "compose") ||
        (query.has("compose") && query.get("compose") !== "1")
      )
        return invalid();
      return {
        kind: "agent",
        project,
        node: parts[3]!,
        view: parts[4] as AgentView,
        compose: query.get("compose") === "1",
      };
    }
    if (parts.length === 4 && !query.size) {
      if (parts[2] === "hosted")
        return { kind: "hosted-agent", project, node: parts[3]! };
      if (parts[2] === "hosts")
        return { kind: "host", project, host: parts[3]! };
      if (parts[2] === "threads")
        return { kind: "thread", project, thread: parts[3]! };
      if (parts[2] === "chat" && parts[3] === "connections")
        return { kind: "chat-connections", project };
      if (parts[2] === "chat")
        return { kind: "chat", project, thread: parts[3]! };
      if (parts[2] === "tasks")
        return { kind: "task", project, task: parts[3]! };
    }
    return invalid();
  } catch {
    return invalid();
  }
}
export function safeReturnPath(href: string, origin: string): string | null {
  if (parseRoute(href, origin).kind === "invalid") return null;
  const url = new URL(href, origin);
  return `${url.pathname}${url.search}`;
}
export function routeSurface(route: Route): Surface {
  if (route.kind === "global") return route.surface;
  if (route.kind === "hosted-agent") return "hosted";
  if (route.kind === "project-settings") return "settings";
  if (route.kind === "chat" || route.kind === "chat-connections") return "chat";
  if (
    route.kind === "project" ||
    route.kind === "project-create" ||
    route.kind === "task"
  )
    return "projects";
  if (
    route.kind === "host" ||
    route.kind === "agent" ||
    route.kind === "thread"
  )
    return "fleet";
  return "home";
}
