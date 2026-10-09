export type Surface = "home" | "fleet" | "projects" | "admin" | "settings";
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
export type AdminView = "users" | "projects" | "settings";
export type Route =
  | { kind: "global"; surface: Surface; project?: string; view?: AdminView }
  | { kind: "project"; project: string; view: ProjectView }
  | { kind: "host"; project: string; host: string }
  | {
      kind: "agent";
      project: string;
      node: string;
      view: AgentView;
      compose: boolean;
    }
  | { kind: "thread"; project: string; thread: string }
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
const adminViews = new Set(["users", "projects", "settings"]);
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
export const projectHref = (project: string, view: ProjectView = "overview") =>
  `/projects/${id(project)}/${view}`;
export const hostHref = (project: string, host: string) =>
  `/projects/${id(project)}/hosts/${id(host)}`;
export const agentHref = (
  project: string,
  node: string,
  view: AgentView = "overview",
  compose = false,
) =>
  `/projects/${id(project)}/agents/${id(node)}/${view}${compose ? "?compose=1" : ""}`;
export const threadHref = (project: string, thread: string) =>
  `/projects/${id(project)}/threads/${id(thread)}`;
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
    if (
      !parts.length ||
      (parts.length === 1 &&
        ["fleet", "projects", "admin", "settings"].includes(first!)) ||
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
    if (first !== "projects" || !parts[1]) return invalid();
    const project = parts[1];
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
      if (parts[2] === "hosts")
        return { kind: "host", project, host: parts[3]! };
      if (parts[2] === "threads")
        return { kind: "thread", project, thread: parts[3]! };
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
  if (route.kind === "project" || route.kind === "task") return "projects";
  if (
    route.kind === "host" ||
    route.kind === "agent" ||
    route.kind === "thread"
  )
    return "fleet";
  return "home";
}
