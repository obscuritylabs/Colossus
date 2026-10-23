// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatSettings } from "./ChatSettings";
import { connectionNetworkPolicy } from "./ConnectionNetworkSettings";
import {
  settingsWrite,
  type ChatSettingsSnapshot,
  type ChatSettingsWrite,
} from "./chat-settings";
import type { SettingsEditorNavigation } from "./settings-editor";

let root: Root, container: HTMLDivElement;
let settings: ChatSettingsSnapshot;
let writes: ChatSettingsWrite[];
let rejection: number | null;
const saved = vi.fn(),
  dirty = vi.fn(),
  close = vi.fn(),
  open = vi.fn();
const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), { status });
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  writes = [];
  rejection = null;
  settings = {
    revision: 7,
    providers: {
      profiles: {
        company: {
          kind: "openai_responses",
          baseUrl: "https://models.example/v1",
        },
      },
    },
    models: {
      profiles: {
        general: {
          providerProfile: "company",
          model: "reasoning-model",
          contextWindowTokens: 128000,
          maxOutputTokens: 16000,
        },
      },
      roles: { primary: "general" },
    },
    mcp: {
      servers: {
        tools: {
          transport: "streamable_http",
          url: "https://mcp.example/mcp",
          allowedTools: ["search"],
        },
      },
    },
    search: {
      profiles: {
        web: { kind: "searxng", endpoint: "https://search.example/search" },
      },
      roles: { agent: "web" },
    },
    provider_authentication: { company: "personal" },
    mcp_authentication: { tools: "personal" },
    credential_status: { providers: {}, mcp: {} },
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_path: string, options: RequestInit) => {
      if (options.method === "GET") return json(settings);
      const body = JSON.parse(String(options.body)) as ChatSettingsWrite;
      writes.push(body);
      if (rejection)
        return json(
          { error: rejection === 409 ? "conflict" : "invalid_argument" },
          rejection,
        );
      settings = {
        ...settings,
        ...body,
        revision: settings.revision + 1,
      } as ChatSettingsSnapshot;
      return json({ revision: settings.revision });
    }),
  );
  vi.stubGlobal(
    "confirm",
    vi.fn(() => true),
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function render(
  navigation: SettingsEditorNavigation = {},
  section: "network" | "providers" | "models" | "mcp" | "search" = "network",
  canAdminister = true,
) {
  await act(() =>
    root.render(
      <ChatSettings
        project="p"
        identity="admin"
        canAdminister={canAdminister}
        section={section}
        onClose={() => {}}
        onSaved={saved}
        onDirtyChange={dirty}
        {...navigation}
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
async function chooseRestricted() {
  await act(() => button("Connection access mode").click());
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((item) => item.textContent?.trim() === "Restrict to an origin list")!;
  await act(() => option.click());
}
async function input(value: string) {
  const field = container.querySelector<HTMLTextAreaElement>(
    'textarea[aria-label="Allowed connection origins"]',
  )!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function submit() {
  await act(() =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}

it("defaults old snapshots to allow all and opens a full URL editor without changing a policy", async () => {
  await render({ onOpenEditor: open });
  expect(container.textContent).toContain("Allow all configured connections");
  expect(container.textContent).toContain(
    "tool network permissions are managed separately",
  );
  await act(() => button("Configure network access").click());
  expect(open).toHaveBeenCalledWith({ action: "edit" });
  expect(writes).toEqual([]);
  expect(settingsWrite(settings)).not.toHaveProperty(
    "connection_network_policy",
  );
});

it("saves a project origin policy with CAS revision, retains unrelated configuration, and waits for canonical readback", async () => {
  await render({ editorLocation: { action: "edit" }, onCloseEditor: close });
  expect(container.querySelector("h2")?.textContent).toBe(
    "Edit network access",
  );
  expect(button("Save network access").disabled).toBe(true);
  await chooseRestricted();
  await input(
    " https://models.example \nhttps://mcp.example:8443\nhttp://localhost:11434\nhttps://models.example\n",
  );
  expect(dirty).toHaveBeenLastCalledWith(true);
  await submit();
  expect(writes).toHaveLength(1);
  expect(writes[0]).toMatchObject({
    revision: 7,
    providers: settings.providers,
    models: settings.models,
    provider_authentication: { company: "personal" },
    connection_network_policy: {
      mode: "allow_list",
      allowed_origins: [
        "https://models.example",
        "https://mcp.example:8443",
        "http://localhost:11434",
      ],
    },
  });
  expect(writes[0]).not.toHaveProperty("credential_status");
  expect(writes[0]).not.toHaveProperty("connection_availability");
  expect(saved).toHaveBeenCalledOnce();
  expect(close).toHaveBeenCalledOnce();
  expect(dirty).toHaveBeenLastCalledWith(false);
  expect(container.textContent).toContain("Connection network policy saved.");
});

it.each([
  "https://example.com/",
  "https://example.com/path",
  "https://user:pass@example.com",
  "https://example.com?",
  "https://example.com#",
  "https://*.example.com",
  "ftp://example.com",
  "https://EXAMPLE.com",
  "https://example.com:443",
  "not a url",
])("rejects noncanonical origin %s before any PUT", async (origin) => {
  await render({ editorLocation: { action: "edit" } });
  await chooseRestricted();
  await input(origin);
  await submit();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "Invalid origin:",
  );
  expect(writes).toEqual([]);
});

it("allows an explicitly empty restricted list and explains its effect", async () => {
  await render({ editorLocation: { action: "edit" } });
  await chooseRestricted();
  expect(container.textContent).toContain(
    "An empty list blocks every connection that requires an origin.",
  );
  await submit();
  expect(writes[0]?.connection_network_policy).toEqual({
    mode: "allow_list",
    allowed_origins: [],
  });
});

it("retains the form on a CAS conflict and requires an explicit reload before resubmitting", async () => {
  rejection = 409;
  await render({ editorLocation: { action: "edit" } });
  await chooseRestricted();
  await input("https://models.example");
  await submit();
  expect(container.querySelector("textarea")?.value).toBe(
    "https://models.example",
  );
  expect(container.textContent).toContain(
    "Reload settings before saving again",
  );
  expect(button("Save network access").disabled).toBe(true);
  await submit();
  expect(writes).toHaveLength(1);
  rejection = null;
  await act(() => button("Reload settings").click());
  expect(container.textContent).toContain("Current settings loaded.");
  expect(container.querySelector("textarea")).toBeNull();
  expect(dirty).toHaveBeenLastCalledWith(false);
});

it("preserves restrictions on unrelated writes and strips computed network metadata", () => {
  settings.connection_network_policy = {
    mode: "allow_list",
    allowed_origins: ["https://models.example"],
  };
  settings.connection_availability = {
    providers: {},
    models: {},
    mcp: {},
    search: {},
    roles: {},
  };
  const copy = settingsWrite(settings);
  expect(copy.connection_network_policy).toEqual(
    settings.connection_network_policy,
  );
  expect(copy).not.toHaveProperty("connection_availability");
  copy.connection_network_policy!.allowed_origins.push("https://other.example");
  expect(settings.connection_network_policy.allowed_origins).toEqual([
    "https://models.example",
  ]);
});

it("reports blocked connections, models, and routes without suggesting credential changes grant access", async () => {
  settings.connection_network_policy = {
    mode: "allow_list",
    allowed_origins: [],
  };
  settings.connection_availability = {
    providers: {
      company: {
        enabled: false,
        reason: "Provider blocked by project Network access.",
      },
    },
    models: {
      general: { enabled: false, reason: "Model provider is blocked." },
    },
    mcp: {
      tools: {
        enabled: false,
        reason: "MCP blocked by project Network access.",
      },
    },
    search: {
      web: {
        enabled: false,
        reason: "Search blocked by project Network access.",
      },
    },
    roles: {
      primary: { enabled: false, reason: "Primary model is unavailable." },
      "search:agent": {
        enabled: false,
        reason: "Search route is unavailable.",
      },
    },
  };
  await render();
  expect(container.textContent).toContain("No origins are allowed.");
  for (const text of [
    "Provider blocked",
    "Model provider is blocked",
    "MCP blocked",
    "Search blocked",
    "Primary model is unavailable",
    "Search route is unavailable",
  ])
    expect(container.textContent).toContain(text);
  for (const [section, text] of [
    ["providers", "Provider blocked"],
    ["models", "Model provider is blocked"],
    ["mcp", "MCP blocked"],
    ["search", "Search blocked"],
  ] as const) {
    await render({}, section);
    const visible = [
      ...container.querySelectorAll<HTMLElement>('[role="status"]'),
    ].filter((item) => !item.closest("[hidden]"));
    expect(visible.some((item) => item.textContent?.includes(text))).toBe(true);
  }
});

it("keeps project authority and dormant runtime restrictions on direct editor URLs", async () => {
  await render({ editorLocation: { action: "edit" } }, "network", false);
  expect(container.textContent).toContain(
    "Project administration access is required",
  );
  expect(container.querySelector("form")).toBeNull();
  settings.execution = {
    enabled: false,
    settings_editable: false,
    reason: "Operator runtime provisioning is required.",
  };
  await render({ editorLocation: { action: "edit" } });
  expect(container.textContent).toContain(
    "Operator runtime provisioning is required.",
  );
  expect(container.querySelector("form")).toBeNull();
  expect(writes).toEqual([]);
});

it("enforces origin count and length while deduplicating repeated lines like the server", () => {
  const valid = Array.from(
    { length: 256 },
    (_, index) => `https://endpoint${index}.example`,
  );
  expect(
    connectionNetworkPolicy("allow_list", [...valid, valid[0]].join("\n"))
      .allowed_origins,
  ).toEqual(valid);
  expect(() =>
    connectionNetworkPolicy(
      "allow_list",
      [...valid, "https://overflow.example"].join("\n"),
    ),
  ).toThrow("at most 256");
  expect(() =>
    connectionNetworkPolicy(
      "allow_list",
      `https://${"a".repeat(2040)}.example`,
    ),
  ).toThrow("Invalid origin");
  expect(connectionNetworkPolicy("allow_all", "invalid")).toEqual({
    mode: "allow_all",
    allowed_origins: [],
  });
});
