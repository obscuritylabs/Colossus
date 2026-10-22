// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatCredentials } from "./ChatCredentials";
import type { CredentialSnapshot } from "./CredentialStoreSettings";
let root: Root, container: HTMLDivElement;
let data: CredentialSnapshot;
let complete: ((value: Response) => void) | null;
let writes: { path: string; method: string; body: Record<string, unknown> }[];
const saved = vi.fn();
const json = (value: unknown) => new Response(JSON.stringify(value));
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  localStorage.clear();
  sessionStorage.clear();
  saved.mockReset();
  complete = null;
  writes = [];
  data = { revision: 3, credentials: [], connections: [] };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit) => {
      if (options.method === "GET") return json(data);
      writes.push({
        path,
        method: String(options.method),
        body: JSON.parse(String(options.body)),
      });
      return new Promise<Response>((resolve) => {
        complete = resolve;
      });
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(project = "p", identity = "alice", canConnect = true) {
  await act(() =>
    root.render(
      <ChatCredentials
        project={project}
        identity={identity}
        canConnect={canConnect}
        onConnections={() => {}}
        onSaved={saved}
      />,
    ),
  );
}
function button(label: string) {
  const value = [
    ...container.querySelectorAll<HTMLButtonElement>("button"),
  ].find(
    (item) =>
      item.textContent?.trim() === label ||
      item.getAttribute("aria-label") === label,
  );
  if (!value) throw new Error(`Missing button ${label}`);
  return value;
}
async function input(label: string, value: string) {
  const field = [...container.querySelectorAll("label")]
    .find((item) => item.querySelector("span")?.textContent === label)
    ?.querySelector("input");
  if (!field) throw new Error(`Missing input ${label}`);
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function select(label: string, value: string) {
  await act(() => button(label).click());
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((item) => item.textContent?.trim() === value)!;
  await act(() => option.click());
}
async function submit(secret: string) {
  const field = container.querySelector<HTMLInputElement>(
    'input[type="password"]',
  )!;
  field.value = secret;
  await act(() =>
    field
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(field.value).toBe("");
}
it("creates an unused named credential without a provider or MCP configured", async () => {
  await render();
  expect(button("Add credential").disabled).toBe(false);
  expect(
    container.querySelector('[aria-label="Search credentials"]'),
  ).not.toBeNull();
  await act(() => button("Add credential").click());
  await input("Display label", "My key");
  await submit("synthetic-private-token");
  expect(button("Cancel").disabled).toBe(true);
  expect(writes).toEqual([
    {
      path: "/api/projects/p/chat/credential-store/personal",
      method: "POST",
      body: {
        revision: 3,
        label: "My key",
        kind: "api_key",
        secret: "synthetic-private-token",
      },
    },
  ]);
  data = {
    revision: 4,
    credentials: [
      {
        id: "key",
        label: "My key",
        kind: "api_key",
        legacy: false,
        used_by: [],
      },
    ],
    connections: [],
  };
  await act(() => complete!(json({ id: "key", revision: 4 })));
  expect(container.textContent).toContain("Not attached");
  expect(container.querySelector('input[type="password"]')).toBeNull();
  expect(container.innerHTML).not.toContain("synthetic-private-token");
  expect(localStorage.length).toBe(0);
  expect(sessionStorage.length).toBe(0);
  expect(saved).toHaveBeenCalledOnce();
});
it("attaches an existing credential only to the reviewed destination", async () => {
  data.connections = [
    {
      id: "api",
      kind: "provider",
      label: "Company API",
      endpoint: "https://api.example/v1",
      credential_binding: "reviewed-binding",
      credential_id: null,
    },
  ];
  data.credentials = [
    {
      id: "key",
      label: "Saved key",
      kind: "api_key",
      legacy: false,
      used_by: [],
    },
  ];
  await render();
  await act(() => button("Attach existing credential").click());
  await select("Saved credential", "Saved key");
  await select("Credential connection", "Provider: Company API");
  expect(container.textContent).toContain("https://api.example/v1");
  await act(() => button("Attach credential").click());
  expect(writes[0]).toEqual({
    path: "/api/projects/p/chat/credential-store/personal/attachments/provider/api",
    method: "POST",
    body: {
      revision: 3,
      credential_id: "key",
      credential_binding: "reviewed-binding",
    },
  });
  data.revision = 4;
  await act(() => complete!(json({ revision: 4 })));
});
it.each([
  ["q", "bob", true],
  ["p", "alice", false],
] as const)(
  "ignores pending completion after authority changes to %s/%s/%s",
  async (project, identity, canConnect) => {
    await render();
    await act(() => button("Add credential").click());
    await input("Display label", "Alice key");
    await submit("alice-private-token");
    await render(project, identity, canConnect);
    await act(() => complete!(json({ revision: 4 })));
    expect(container.querySelector('input[type="password"]')).toBeNull();
    expect(container.innerHTML).not.toContain("alice-private-token");
    expect(container.textContent).not.toContain("Credential changes saved.");
    expect(saved).not.toHaveBeenCalled();
  },
);
it("clears cancelled entry and disables mutations without connection permission", async () => {
  await render("p", "alice", false);
  expect(button("Add credential").disabled).toBe(true);
  await render();
  await act(() => button("Add credential").click());
  await input("Display label", "Cancelled key");
  const field = container.querySelector<HTMLInputElement>(
    'input[type="password"]',
  )!;
  field.value = "cancelled-token";
  await act(() =>
    field.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    ),
  );
  expect(container.querySelector('input[type="password"]')).toBeNull();
  expect(writes).toEqual([]);
});
