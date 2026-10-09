// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { navigate } from "./navigation";
import type { Me } from "./control-api";
import type { FleetNode, ThreadDetailResponse } from "./api";

const now = "2026-10-06T10:00:00Z";
const me: Me = {
  user: {
    id: "viewer-a",
    display_name: "Viewer",
    email: null,
    active: true,
    is_admin: false,
    revision: 1,
    created_at: now,
    updated_at: now,
  },
  memberships: [
    {
      project_id: "p",
      user_id: "viewer-a",
      subject: "viewer-a",
      role: "viewer",
      permissions: ["read"],
    },
  ],
  projects: [
    {
      id: "p",
      name: "Project P",
      description: "",
      parent_project_id: null,
      archived: false,
      revision: 1,
      created_at: now,
      updated_at: now,
    },
  ],
};
const node: FleetNode = {
  node: {
    node_id: "n",
    project_id: "p",
    label: "Authorized agent",
    instance_id: "instance",
    certificate_sha256: "fingerprint",
    roles: ["primary"],
    revoked: false,
    revision: 1,
  },
  presence: null,
};
const detail: ThreadDetailResponse = {
  thread: {
    thread_id: "t",
    project_id: "p",
    node_id: "n",
    session_id: "s",
    title: "Exact saved conversation",
    created_at: now,
    updated_at: now,
    revision: 1,
    archived: false,
    source: "runtime",
    sync_status: "current",
    can_continue: false,
  },
  tasks: [],
  messages: [
    {
      message_id: "message",
      role: "assistant",
      text: "Retained exact-resource reply",
      created_at: now,
    },
  ],
};
class EventSourceFixture extends EventTarget {
  onopen = null;
  onerror = null;
  close() {}
}
let root: Root, container: HTMLDivElement, paths: string[], account: Me;
let showInventoryNode = false,
  denyInventory = false,
  deferInventory = false;
const pendingInventory: { resolve: ((value: Response) => void) | null } = {
  resolve: null,
};
const response = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status });
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState(null, "", "/projects/p/threads/t");
  sessionStorage.clear();
  vi.stubGlobal("EventSource", EventSourceFixture);
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  paths = [];
  account = me;
  showInventoryNode = false;
  denyInventory = false;
  deferInventory = false;
  pendingInventory.resolve = null;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string) => {
      paths.push(path);
      if (path === "/api/me") return response(account);
      if (path === "/api/auth/config")
        return response({ local_enabled: false, oidc: null });
      if (path === "/api/settings")
        return response({
          classification: {
            enabled: false,
            text: "",
            tone: "neutral",
            position: "top",
          },
        });
      if (path === "/api/projects/p/threads/t") return response(detail);
      if (path === "/api/projects/p/nodes/n") return response(node);
      if (path.startsWith("/api/projects/p/nodes?")) {
        if (deferInventory) {
          deferInventory = false;
          return new Promise<Response>((resolve) => {
            pendingInventory.resolve = resolve;
          });
        }
        if (denyInventory) return response({ error: "permission_denied" }, 403);
        return response({
          nodes: showInventoryNode ? [node] : [],
          next_cursor: null,
        });
      }
      if (path.startsWith("/api/projects/p/tasks?"))
        return response({ tasks: [], next_cursor: null });
      if (path.startsWith("/api/projects/p/hosts?"))
        return response({ hosts: [], next_cursor: null });
      if (path.startsWith("/api/projects/p/threads?"))
        return response({ threads: [], next_cursor: null });
      const archivedInventory =
        /^\/api\/projects\/archived\/(nodes|tasks|hosts)\?/.exec(path);
      if (archivedInventory)
        return response({ [archivedInventory[1]!]: [], next_cursor: null });
      if (path.endsWith("/analytics"))
        return response({
          generated_at: now,
          window_days: 7,
          counts: {
            threads: 0,
            runs: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
            active: 0,
            queued: 0,
            agents: 0,
            online_agents: 0,
            hosts: 0,
          },
          activity: [],
          usage: {
            input_tokens: null,
            output_tokens: null,
            estimated_cost: null,
            coverage: "Unavailable",
          },
        });
      return response({ error: "permission_denied" }, 403);
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  history.replaceState(null, "", "/");
  vi.unstubAllGlobals();
});
async function ready() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 15));
  });
}
async function contains(text: string) {
  await vi.waitFor(
    async () => {
      await ready();
      expect(container.textContent).toContain(text);
    },
    { timeout: 2000 },
  );
}
it("reopens an exact saved deep link and uses an authorized parent on direct-link Back", async () => {
  await act(() => root.render(<App />));
  await contains("Exact saved conversation");
  expect(
    container
      .querySelector('a[aria-label="Documentation (opens in a new tab)"]')
      ?.getAttribute("href"),
  ).toBe("/docs/");
  expect(container.textContent).toContain("Retained exact-resource reply");
  expect(paths).toContain("/api/projects/p/threads/t");
  expect(paths).toContain("/api/projects/p/nodes/n");
  const back = container.querySelector<HTMLAnchorElement>(
    'a.back[href="/projects/p/agents/n/overview"]',
  );
  expect(back).not.toBeNull();
  await act(() => back!.click());
  await ready();
  expect(location.pathname).toBe("/projects/p/agents/n/overview");
  expect(container.textContent).toContain("Authorized agent");
});
it("does not fall back to a prior conversation or project after denied navigation", async () => {
  await act(() => root.render(<App />));
  await contains("Retained exact-resource reply");
  await act(() => navigate("/projects/p/threads/denied"));
  await ready();
  expect(container.textContent).toContain("Page unavailable");
  expect(container.textContent).not.toContain("Retained exact-resource reply");
  await act(() => navigate("/projects/unauthorized/threads/t"));
  await ready();
  expect(container.textContent).toContain("does not have access");
  expect(container.textContent).not.toContain("Exact saved conversation");
  expect(
    paths.some((path) => path.startsWith("/api/projects/unauthorized/")),
  ).toBe(false);
});
it("hides denied inventory immediately and rejects an older in-flight permitted response", async () => {
  history.replaceState(null, "", "/fleet?project=p");
  showInventoryNode = true;
  await act(() => root.render(<App />));
  await contains("Authorized agent");
  const refresh = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Refresh project"]',
  )!;
  deferInventory = true;
  await act(() => refresh.click());
  await ready();
  expect(pendingInventory.resolve).not.toBeNull();
  denyInventory = true;
  await act(() => refresh.click());
  await contains("Page unavailable");
  expect(container.textContent).not.toContain("Authorized agent");
  await act(async () =>
    pendingInventory.resolve!(response({ nodes: [node], next_cursor: null })),
  );
  expect(container.textContent).toContain("Page unavailable");
  expect(container.textContent).not.toContain("Authorized agent");
});

it("defaults to an active project while preserving explicitly selected archived scope", async () => {
  history.replaceState(null, "", "/");
  account = {
    ...me,
    projects: [
      {
        ...me.projects[0]!,
        id: "archived",
        name: "Archived project",
        archived: true,
      },
      ...me.projects,
    ],
    memberships: [
      ...me.memberships,
      { ...me.memberships[0]!, project_id: "archived" },
    ],
  };
  await act(() => root.render(<App />));
  await contains("Project P");
  const selector = () =>
    container.querySelector<HTMLButtonElement>(
      'button[aria-label="Select project"]',
    )!;
  expect(selector().textContent).toContain("Project P");
  expect(paths.some((path) => path.startsWith("/api/projects/p/nodes?"))).toBe(
    true,
  );
  expect(paths.some((path) => path.startsWith("/api/projects/archived/"))).toBe(
    false,
  );

  await act(() => selector().click());
  const archivedOption = [
    ...document.querySelectorAll<HTMLButtonElement>('[role="option"]'),
  ].find((option) => option.textContent?.includes("Archived project"));
  expect(archivedOption).toBeDefined();
  await act(() => archivedOption!.click());
  await ready();
  expect(location.search).toBe("?project=archived");
  expect(selector().textContent).toContain("Archived project");

  await act(() => navigate("/fleet?project=archived"));
  await ready();
  expect(selector().textContent).toContain("Archived project");
  await act(() => navigate("/"));
  await ready();
  expect(selector().textContent).toContain("Archived project");
});
