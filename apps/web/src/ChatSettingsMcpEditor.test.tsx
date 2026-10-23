// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatSettingsEditor } from "./ChatSettingsEditors";
import { settingsWrite, type ChatSettingsSnapshot } from "./chat-settings";

let root: Root, container: HTMLDivElement;
let settings: ChatSettingsSnapshot;
const onApply = vi.fn(),
  onPersonalCredentials = vi.fn(),
  onCredentials = vi.fn();
const fetch = vi.fn();

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  vi.stubGlobal("fetch", fetch);
  settings = {
    revision: 3,
    providers: { profiles: {} },
    models: { profiles: {}, roles: {} },
    mcp: { servers: {} },
    provider_authentication: {},
    mcp_authentication: {},
    credential_status: { providers: {}, mcp: {} },
  };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  expect(fetch).not.toHaveBeenCalled();
  vi.unstubAllGlobals();
});

async function renderEditor(id = "", instanceScope = false) {
  await act(() =>
    root.render(
      <ChatSettingsEditor
        key={`${instanceScope}:${id}`}
        editor={{ kind: "mcp", id }}
        draft={settingsWrite(settings)}
        saved={settings}
        project="p"
        instanceScope={instanceScope}
        onApply={onApply}
        onCancel={() => {}}
        canManageCredentials
        onCredentials={onCredentials}
        onPersonalCredentials={onPersonalCredentials}
      />,
    ),
  );
}

async function input(label: string, value: string) {
  const field = container.querySelector<HTMLInputElement>(
    `input[aria-label="${label}"]`,
  )!;
  expect(field).not.toBeNull();
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function newConnection(
  tools = "search",
  url = "https://mcp.example.test/mcp",
) {
  await renderEditor();
  await input("Profile ID", "12112");
  await input("MCP server URL", url);
  await input("Allowed tools", tools);
}

async function submit() {
  await act(() =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}

async function select(label: string, option: string) {
  const control = container.querySelector<HTMLButtonElement>(
    `button[aria-label="${label}"]`,
  )!;
  expect(control).not.toBeNull();
  await act(() => control.click());
  const choice = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((item) => item.textContent?.trim() === option)!;
  expect(choice).toBeDefined();
  await act(() => choice.click());
}

async function oauthConnection(client: string, scopes: string) {
  await newConnection();
  await select("Sign-in method", "OAuth sign-in");
  await input("OAuth client ID", client);
  await input("OAuth scopes", scopes);
}

it.each([
  ["missing client ID", "", "read", "registered OAuth client ID"],
  ["oversized ASCII client ID", "a".repeat(1025), "read", "1,024 UTF-8 bytes"],
  ["oversized UTF-8 client ID", "é".repeat(513), "read", "1,024 UTF-8 bytes"],
  [
    "control in client ID",
    "valid\u0000client",
    "read",
    "without control characters",
  ],
  [
    "too many scopes",
    "registered-client",
    Array.from({ length: 33 }, (_, index) => `scope_${index}`).join(", "),
    "at most 32 OAuth scopes",
  ],
  [
    "oversized ASCII scope",
    "registered-client",
    "a".repeat(257),
    "256 UTF-8 bytes",
  ],
  [
    "oversized UTF-8 scope",
    "registered-client",
    "é".repeat(129),
    "256 UTF-8 bytes",
  ],
  [
    "control in scope",
    "registered-client",
    "read\u0000documents",
    "without control characters or spaces",
  ],
])(
  "rejects OAuth %s before applying or making a request",
  async (_reason, client, scopes, message) => {
    await oauthConnection(client, scopes);
    await submit();
    expect(onApply).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      message,
    );
  },
);

it("accepts OAuth UTF-8 boundaries and preserves private sign-in without static credential setup", async () => {
  const client = "é".repeat(512);
  const scopes = [
    "é".repeat(128),
    ...Array.from({ length: 31 }, (_, index) => `scope_${index}`),
  ];
  await oauthConnection(client, scopes.join(", "));
  expect(container.textContent).not.toContain("Save and choose credential");
  await submit();
  expect(onApply).toHaveBeenCalledOnce();
  const [configuration, setup] = onApply.mock.calls[0]!;
  expect(configuration.mcp.servers["12112"]).toMatchObject({
    oauth: { clientId: client, scopes, callbackPort: 8787 },
    credentialHeaders: {},
  });
  expect(configuration.mcp_authentication["12112"]).toBe("personal");
  expect(setup).toBeUndefined();
  expect(onCredentials).not.toHaveBeenCalled();
  expect(onPersonalCredentials).not.toHaveBeenCalled();
});

it("rejects shared OAuth authentication with a field-specific correction", async () => {
  await oauthConnection("registered-client", "read");
  await select("Authentication", "Shared credential (explicit assignment)");
  await submit();
  expect(onApply).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "OAuth connections use each user's personal sign-in",
  );
});

it.each([
  ["missing selectors", ""],
  [
    "too many selectors",
    Array.from({ length: 1025 }, (_, index) => `tool_${index}`).join(", "),
  ],
  ["oversized selector", "a".repeat(129)],
  ["consecutive stars", "get_**"],
  ["regular expression", "^get_.*$"],
  ["question mark", "get_?"],
  ["character class", "get_[ab]"],
  ["escape", "get_\\*"],
  ["non-ASCII selector", "gét_*"],
  ["control character", "read\u0000document"],
])("rejects %s before applying MCP settings", async (_reason, tools) => {
  await newConnection(tools);
  await submit();
  expect(onApply).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "exact tool names or * patterns",
  );
});

it.each([
  ["mixed all-tools wildcard", "*, search", "Use * alone"],
  ["duplicate selectors", "get_*, get_*", "only once"],
])(
  "rejects %s without changing the selector list",
  async (_reason, tools, message) => {
    await newConnection(tools);
    await submit();
    expect(onApply).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      message,
    );
  },
);

it.each([
  ["*", ["*"]],
  [
    "search, get_*, *_search, filesystem.*",
    ["search", "get_*", "*_search", "filesystem.*"],
  ],
  ["a".repeat(128), ["a".repeat(128)]],
])("accepts supported MCP selectors %s", async (tools, expected) => {
  await newConnection(tools);
  await submit();
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply.mock.calls[0]![0].mcp.servers["12112"].allowedTools).toEqual(
    expected,
  );
});

it("accepts the 1,024-selector limit", async () => {
  const selectors = Array.from({ length: 1024 }, (_, index) => `tool_${index}`);
  await newConnection(selectors.join(", "));
  await submit();
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply.mock.calls[0]![0].mcp.servers["12112"].allowedTools).toEqual(
    selectors,
  );
});

it.each([
  ["*", ["*"]],
  ["search, read_document", ["search", "read_document"]],
])(
  "preserves personal credential setup for selectors %s",
  async (tools, expected) => {
    await newConnection(tools);
    const chooseCredential = [...container.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "Save and choose credential",
    )!;
    await act(() => chooseCredential.click());
    expect(onApply).toHaveBeenCalledOnce();
    expect(onApply.mock.calls[0]![0]).toMatchObject({
      mcp: {
        servers: {
          "12112": {
            allowedTools: expected,
            credentialHeaders: {
              Authorization: { scheme: "Bearer", reference: "host:managed" },
            },
          },
        },
      },
      mcp_authentication: { "12112": "personal" },
    });
    expect(onApply.mock.calls[0]![1]).toEqual({
      target: { kind: "mcp", id: "12112" },
      authentication: "personal",
    });
    expect(onCredentials).not.toHaveBeenCalled();
    expect(onPersonalCredentials).not.toHaveBeenCalled();
  },
);

it("provides concise field help and expandable pattern details", async () => {
  await newConnection();
  for (const [label, expected] of [
    ["Allowed tools", "Use * alone for all tools"],
    ["MCP server URL", "Localhost refers to the Control Plane server"],
  ]) {
    const field = container.querySelector<HTMLInputElement>(
      `input[aria-label="${label}"]`,
    )!;
    const help = document.getElementById(
      field.getAttribute("aria-describedby")!,
    );
    expect(help?.textContent).toContain(expected);
  }
  const details =
    container.querySelector<HTMLDetailsElement>(".mcp-pattern-help")!;
  expect(details.open).toBe(false);
  expect(details.querySelector("summary")?.textContent).toBe(
    "Pattern matching",
  );
  expect(details.textContent).toContain(
    "Regex and consecutive stars are unsupported",
  );
});

it.each([
  "http://localhost:8090/mcp",
  "http://127.0.0.2:8090/mcp",
  "http://[::1]:8090/mcp",
])("accepts a new instance loopback endpoint %s", async (url) => {
  await renderEditor("", true);
  await input("Profile ID", "local_tools");
  await input("MCP server URL", url);
  await input("Allowed tools", "*");
  await submit();
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply.mock.calls[0]![0].mcp.servers.local_tools.url).toBe(url);
});

it("requires HTTPS for remote instance endpoints", async () => {
  await renderEditor("", true);
  await input("Profile ID", "remote_tools");
  await input("MCP server URL", "http://mcp.example.test/mcp");
  await input("Allowed tools", "*");
  await submit();
  expect(onApply).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "HTTPS MCP endpoint, or HTTP for a loopback connection",
  );
});

it("explains invalid Profile IDs before applying", async () => {
  await newConnection();
  await input("Profile ID", "docs.remote");
  await submit();
  expect(onApply).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "nonreserved Profile ID of 1–128",
  );
});

it.each(["http://google.com", "http://mcp.example.test/mcp"])(
  "rejects a new remote HTTP MCP endpoint %s before applying",
  async (url) => {
    await newConnection("search", url);
    await submit();
    expect(onApply).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Use an HTTPS MCP endpoint",
    );
  },
);

it.each([
  "https://user:password@mcp.example.test/mcp",
  "https://mcp.example.test/mcp?token=private",
  "https://mcp.example.test/mcp?",
  "https://mcp.example.test/mcp#fragment",
  "https://mcp.example.test/mcp#",
])("rejects unsupported endpoint components in %s", async (url) => {
  await newConnection("search", url);
  await submit();
  expect(onApply).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "without embedded credentials, a query, or a fragment",
  );
});

it.each([
  "http://localhost:8090/mcp",
  "http://127.0.0.2:8090/mcp",
  "http://[::1]:8090/mcp",
])("supports new and edited project loopback endpoints %s", async (url) => {
  await newConnection("search", url);
  await submit();
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply.mock.calls[0]![0].mcp.servers["12112"].url).toBe(url);
  onApply.mockClear();
  settings.mcp.servers.docs = {
    transport: "streamable_http",
    url,
    allowedTools: ["search"],
  };
  settings.mcp_authentication.docs = "personal";
  await renderEditor("docs");
  await submit();
  expect(onApply).toHaveBeenCalledOnce();
  expect(onApply.mock.calls[0]![0].mcp.servers.docs.url).toBe(url);
  await input("MCP server URL", `${url}/changed`);
  await submit();
  expect(onApply).toHaveBeenCalledTimes(2);
  expect(onApply.mock.calls[1]![0].mcp.servers.docs.url).toBe(`${url}/changed`);
});
