// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatProviderSettings } from "./ChatProviderSettings";
import { settingsWrite, type ChatSettingsSnapshot } from "./chat-settings";

let root: Root, container: HTMLDivElement;
const apply = vi.fn();
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify({ presets: [] }))),
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function input(label: string, value: string) {
  const field = [...container.querySelectorAll("label")]
    .find((item) => item.querySelector("span")?.textContent === label)
    ?.querySelector("input");
  if (!field) throw new Error(`Missing ${label}`);
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
it.each([
  [true, false, "http://localhost:11434/v1", true],
  [true, false, "http://127.0.0.2:1234/v1", true],
  [true, false, "http://[::1]:1234/v1", true],
  [true, false, "http://provider.example/v1", false],
  [false, false, "http://localhost:11434/v1", true],
  [false, true, "http://localhost:11434/v1", true],
  [false, false, "https://provider.example/v1", true],
  [true, false, "https://provider.example/v1?", false],
  [true, false, "https://provider.example/v1#", false],
  [true, false, "https://user:pass@provider.example/v1", false],
])(
  "validates provider transport in instance=%s existing=%s endpoint=%s",
  async (instanceScope, existing, endpoint, accepted) => {
    const snapshot: ChatSettingsSnapshot = {
      revision: 7,
      providers: {
        profiles: existing
          ? { provider: { kind: "openai_responses", baseUrl: endpoint } }
          : {},
      },
      models: { profiles: {}, roles: {} },
      mcp: { servers: {} },
      provider_authentication: {},
      mcp_authentication: {},
      credential_status: { providers: {}, mcp: {} },
    };
    await act(() =>
      root.render(
        <ChatProviderSettings
          id={existing ? "provider" : ""}
          instanceScope={instanceScope}
          draft={settingsWrite(snapshot)}
          saved={snapshot}
          project="p"
          onApply={apply}
          onCancel={() => {}}
        />,
      ),
    );
    if (!existing) {
      await input("Display label", "Provider");
      await input("Profile ID", "provider");
      await input("Endpoint URL", endpoint);
    }
    expect(container.textContent).toContain(
      "HTTP loopback addresses connect to the Control Plane server.",
    );
    await act(() =>
      container
        .querySelector("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        ),
    );
    expect(apply).toHaveBeenCalledTimes(accepted ? 1 : 0);
    if (accepted)
      expect(apply.mock.calls[0]?.[0].providers.profiles.provider.baseUrl).toBe(
        endpoint,
      );
    else
      expect(container.querySelector('[role="alert"]')?.textContent).toMatch(
        /HTTPS|credentials/,
      );
  },
);

it.each([
  ["", false],
  ["é".repeat(49), false],
  ["Provider\u0000label", false],
  ["Provider\u0085label", false],
  ["p".repeat(96), true],
  ["é".repeat(48), true],
] as const)(
  "validates provider display labels before applying (%s)",
  async (label, accepted) => {
    const snapshot: ChatSettingsSnapshot = {
      revision: 7,
      providers: { profiles: {} },
      models: { profiles: {}, roles: {} },
      mcp: { servers: {} },
      provider_authentication: {},
      mcp_authentication: {},
      credential_status: { providers: {}, mcp: {} },
    };
    await act(() =>
      root.render(
        <ChatProviderSettings
          id=""
          draft={settingsWrite(snapshot)}
          saved={snapshot}
          project="p"
          onApply={apply}
          onCancel={() => {}}
        />,
      ),
    );
    await input("Display label", label);
    await input("Profile ID", "provider");
    await input("Endpoint URL", "https://provider.example/v1");
    await act(() =>
      container
        .querySelector("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        ),
    );
    expect(apply).toHaveBeenCalledTimes(accepted ? 1 : 0);
    if (accepted)
      expect(apply.mock.calls[0]?.[0].labels.providers.provider).toBe(label);
    else
      expect(container.querySelector('[role="alert"]')?.textContent).toContain(
        "1–96 UTF-8 bytes and no control characters",
      );
  },
);
