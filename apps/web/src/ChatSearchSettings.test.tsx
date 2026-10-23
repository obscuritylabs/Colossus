// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ConnectionSettingsExtras } from "./ConnectionSettingsExtras";
import { settingsWrite, type ChatSettingsSnapshot } from "./chat-settings";

let root: Root, container: HTMLDivElement;
const save = vi.fn();
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  vi.stubGlobal("fetch", vi.fn());
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
async function input(label: string, value: string) {
  const field = [...container.querySelectorAll("label")]
    .find((element) => element.querySelector("span")?.textContent === label)
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
async function select(label: string, option: string) {
  const control =
    container.querySelector<HTMLButtonElement>(
      `button[aria-label="${label}"]`,
    ) ??
    [...container.querySelectorAll("label")]
      .find((element) => element.querySelector("span")?.textContent === label)
      ?.querySelector<HTMLButtonElement>('button[role="combobox"]');
  if (!control) throw new Error(`Missing select ${label}`);
  await act(() => control.click());
  const choice = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((element) => element.textContent?.trim() === option);
  if (!choice) throw new Error(`Missing option ${option}`);
  await act(() => choice.click());
}
async function render(instanceScope = false) {
  const snapshot: ChatSettingsSnapshot = {
    revision: 7,
    providers: { profiles: {} },
    models: { profiles: {}, roles: {} },
    mcp: { servers: {} },
    search: { profiles: {}, roles: {} },
    provider_authentication: {},
    mcp_authentication: {},
    credential_status: { providers: {}, mcp: {} },
  };
  await act(() =>
    root.render(
      <ConnectionSettingsExtras
        section="search"
        draft={settingsWrite(snapshot)}
        saved={snapshot}
        instanceScope={instanceScope}
        busy={false}
        editorLocation={{ action: "new" }}
        onChange={() => {}}
        onSave={save}
      />,
    ),
  );
  await input("Display label", "Search");
  await input("Profile ID", "web");
  await input("Endpoint URL", "https://search.example/search");
}
async function submit() {
  await act(() =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}
it.each([
  ["", false],
  ["http://search.example/search", false],
  ["https://user:pass@search.example/search", false],
  ["https://search.example/search?", false],
  ["https://search.example/search?format=json", false],
  ["https://search.example/search#", false],
  ["https://search.example/search#section", false],
  ["https://search.example/not-search", false],
  ["https://search.example/search", true],
  ["http://localhost:8080/search", true],
  ["http://127.2.3.4:8080/search", true],
  ["http://[::1]:8080/search", true],
] as const)(
  "validates search transport and endpoint grammar before saving (%s)",
  async (endpoint, accepted) => {
    await render();
    await input("Endpoint URL", endpoint);
    expect(container.textContent).toContain(
      "HTTP loopback addresses connect to the Control Plane server.",
    );
    await submit();
    expect(save).toHaveBeenCalledTimes(accepted ? 1 : 0);
    if (accepted)
      expect(save.mock.calls[0]?.[0].search.profiles.web.endpoint).toBe(
        endpoint,
      );
    else expect(container.querySelector('[role="alert"]')).not.toBeNull();
  },
);
it.each([false, true])(
  "accepts bounded search labels and request controls in instance=%s",
  async (instanceScope) => {
    await render(instanceScope);
    const label = "é".repeat(48),
      authHeader = "x".repeat(128),
      userAgent = "é".repeat(128);
    await input("Display label", label);
    await input("Authentication header", authHeader);
    await input("User agent", userAgent);
    await submit();
    expect(save).toHaveBeenCalledTimes(1);
    expect(save.mock.calls[0]?.[0]).toMatchObject({
      labels: { search: { web: label } },
      search: { profiles: { web: { authHeader, userAgent } } },
    });
  },
);
it.each([
  ["Display label", "", "1–96 UTF-8 bytes"],
  ["Display label", "é".repeat(49), "1–96 UTF-8 bytes"],
  ["Display label", "Invalid\u0000label", "control characters"],
  ["Display label", "Invalid\u0085label", "control characters"],
  ["Authentication header", "x".repeat(129), "1–128 ASCII HTTP header-name"],
  ["Authentication header", "Header:Name", "HTTP header-name"],
  ["Authentication header", "Header Name", "HTTP header-name"],
  ["Authentication header", "é", "HTTP header-name"],
  ["Authentication header", "Header\u0000Name", "HTTP header-name"],
  ["User agent", "", "1–256 UTF-8 bytes"],
  ["User agent", "é".repeat(129), "1–256 UTF-8 bytes"],
  ["User agent", "Client\u0000name", "ASCII control"],
])(
  "explains invalid search %s before saving",
  async (label, value, message) => {
    await render();
    await input(label, value);
    await submit();
    expect(save).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      message,
    );
  },
);
it("preserves the default SearXNG header when left blank", async () => {
  await render();
  await input("Authentication header", "");
  await submit();
  expect(save.mock.calls[0]?.[0].search.profiles.web.authHeader).toBe(
    "X-Searxng-Key",
  );
});
it("uses SerpAPI personal authentication and persists its visible user agent", async () => {
  await render(true);
  await select("Adapter", "SerpAPI");
  expect(
    container.querySelector('button[aria-label="Search authentication"]')
      ?.textContent,
  ).toContain("Personal credential (each user)");
  await input("Endpoint URL", "https://serpapi.com/search.json");
  await input("User agent", "Organization Search/2.0");
  await select("Search authentication", "No authentication");
  await submit();
  expect(save).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "SerpAPI requires a personal or explicitly shared credential",
  );
  await select("Search authentication", "Personal credential (each user)");
  await submit();
  expect(save).toHaveBeenCalledTimes(1);
  expect(save.mock.calls[0]?.[0]).toMatchObject({
    search_authentication: { web: "personal" },
    search: {
      profiles: {
        web: {
          kind: "serp_api",
          credentialReference: "host:managed",
          userAgent: "Organization Search/2.0",
        },
      },
    },
  });
  expect(save.mock.calls[0]?.[0].search.profiles.web).not.toHaveProperty(
    "authHeader",
  );
});
