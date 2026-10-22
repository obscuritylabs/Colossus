// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Chat, personalChatPermissions } from "./Chat";
import { NavigationProvider } from "./navigation";
import type { FleetNode, Permission } from "./api";

const node: FleetNode = {
  node: {
    node_id: "personal-node",
    project_id: "p",
    instance_id: "personal-instance",
    label: "Cloud chat",
    certificate_sha256: "certificate",
    roles: ["primary"],
    revoked: false,
    revision: 1,
  },
  presence: { ready: true, connection_id: "connection", capabilities: [] },
};
const json = (value: unknown) =>
  new Response(JSON.stringify(value), { status: 200 });
let root: Root, container: HTMLDivElement;
let connected: boolean,
  authenticated: boolean,
  enabled: boolean,
  authUrl: string;
let requests: { path: string; body: unknown }[];
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState(null, "", "/chat?project=p");
  connected = false;
  authenticated = false;
  enabled = true;
  authUrl = "https://identity.example/authorize?state=synthetic";
  requests = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit) => {
      if (options.method === "POST") {
        const body = JSON.parse(String(options.body));
        requests.push({ path, body });
        if (path.endsWith("/connect")) {
          connected = true;
          return json({ enabled, node });
        }
        if (path.endsWith("/auth")) return json({ authorization_url: authUrl });
        if (path.endsWith("/disconnect")) {
          authenticated = false;
          return json({});
        }
        if (path.endsWith("/threads"))
          return json({
            thread: { project_id: "p", thread_id: "personal-thread" },
          });
        if (path.endsWith("/resources"))
          return json({
            kind: "result",
            connection_id: "connection",
            value: { capabilities: [] },
          });
        throw new Error(`Unexpected mutation ${path}`);
      }
      if (path === "/api/projects/p/chat")
        return json({ enabled, node: connected ? node : null });
      if (path.endsWith("/providers")) return json({ providers: [] });
      if (path.includes("/chat/threads?"))
        return json({ threads: [], next_cursor: null });
      if (path.endsWith("/connections"))
        return json({
          connections: [
            {
              id: "personal",
              label: "Personal source",
              transport: "streamable_http",
              authentication: "personal",
              authenticated,
              oauth: true,
              allowed_tools: ["read"],
            },
            {
              id: "shared",
              label: "Team source",
              transport: "streamable_http",
              authentication: "shared",
              authenticated: true,
              oauth: false,
              allowed_tools: ["search"],
            },
          ],
        });
      throw new Error(`Unexpected read ${path}`);
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(
  permissions: Permission[] = ["read", "execute", "control"],
  view: "conversation" | "connections" = "conversation",
) {
  await act(async () =>
    root.render(
      <NavigationProvider>
        <Chat
          project="p"
          identity="user-a"
          permissions={permissions}
          view={view}
          selected={null}
          detailLoading={false}
          onChanged={() => {}}
        />
      </NavigationProvider>,
    ),
  );
}
function button(text: string) {
  const value = [
    ...container.querySelectorAll<HTMLButtonElement>("button"),
  ].find((item) => item.textContent?.trim() === text);
  if (!value) throw new Error(`Missing button ${text}`);
  return value;
}
async function write(text: string) {
  const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("starts a server-assigned personal runtime and submits without caller-selected node or owner", async () => {
  await render();
  expect(container.querySelector(".chat-history")).toBeNull();
  expect(
    container.querySelectorAll(
      ".chat-conversation-workspace .shared-work-header",
    ),
  ).toHaveLength(1);
  expect(
    container.querySelector('button[aria-label="Execution agent"]'),
  ).toBeNull();
  await act(() => button("Start chat").click());
  await write("A personal cloud question");
  await act(() =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  const submission = requests.find(
    (item) => item.path === "/api/projects/p/chat/threads",
  )!;
  expect(submission.body).toMatchObject({
    request: { input: [{ text: "A personal cloud question" }] },
  });
  expect(submission.body).not.toHaveProperty("node_id");
  expect(submission.body).not.toHaveProperty("user_id");
  expect(location.pathname).toBe("/projects/p/chat/personal-thread");
});

it("keeps personal OAuth controls distinct from an explicit project connection", async () => {
  connected = true;
  await render(undefined, "connections");
  expect(
    container.querySelector('section[aria-label="Team source"]')?.textContent,
  ).toContain("Project connection");
  expect(
    container.querySelector('section[aria-label="Team source"] button'),
  ).toBeNull();
  await act(() => button("Connect").click());
  expect(
    container.querySelector<HTMLAnchorElement>(".chat-authorization a")?.href,
  ).toBe(authUrl);
  expect(
    container.querySelector(".chat-authorization a")?.getAttribute("rel"),
  ).toContain("noopener");
  authenticated = true;
  await act(() => root.unmount());
  root = createRoot(container);
  await render(undefined, "connections");
  await act(() => button("Disconnect").click());
  expect(button("Connect").disabled).toBe(false);
  expect(
    requests
      .filter((item) => item.path.includes("/connections/"))
      .map((item) => item.path),
  ).toEqual([
    "/api/projects/p/chat/connections/personal/auth",
    "/api/projects/p/chat/connections/personal/disconnect",
  ]);
});

it("rejects unsafe authorization navigation and respects execution and configuration availability", async () => {
  connected = true;
  authUrl = "https://credential@evil.example/authorize";
  await render(["read"], "connections");
  expect(button("Connect").disabled).toBe(true);
  expect(
    container.querySelector<HTMLTextAreaElement>("textarea")?.disabled,
  ).toBe(true);
  await render(undefined, "connections");
  await act(() => button("Connect").click());
  expect(container.textContent).toContain("invalid sign-in address");
  expect(container.querySelector(".chat-authorization a")).toBeNull();
  enabled = false;
  await act(() => root.unmount());
  root = createRoot(container);
  await render();
  expect(container.textContent).toContain("Cloud chat is not configured");
  expect(
    container.querySelector<HTMLTextAreaElement>("textarea")?.disabled ?? true,
  ).toBe(true);
  expect(
    [...container.querySelectorAll("button")].some(
      (item) => item.textContent === "Start chat",
    ),
  ).toBe(false);
});

it("shows personal approval authority only for the exact owned personal node with execution and control", () => {
  const own: Permission[] = ["read", "execute", "control"];
  expect(personalChatPermissions(own, "user-a", "user-a", true)).toContain(
    "approve",
  );
  expect(
    personalChatPermissions(own, "other-user", "user-a", true),
  ).not.toContain("approve");
  expect(personalChatPermissions(own, "user-a", "user-a", false)).not.toContain(
    "approve",
  );
  expect(
    personalChatPermissions(["read", "execute"], "user-a", "user-a", true),
  ).not.toContain("approve");
  expect(
    personalChatPermissions(["read"], "user-a", "user-a", true),
  ).not.toContain("approve");
  expect(personalChatPermissions(own, undefined, "user-a", true)).not.toContain(
    "approve",
  );
});
