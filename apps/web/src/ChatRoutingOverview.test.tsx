// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatSettings } from "./ChatSettings";
import type { ChatSettingsSnapshot } from "./chat-settings";

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const settings: ChatSettingsSnapshot = {
    revision: 1,
    providers: { profiles: { company: { kind: "openai_responses" } } },
    models: {
      profiles: {
        general: {
          providerProfile: "company",
          model: "general-model",
          contextWindowTokens: 128000,
          maxOutputTokens: 16000,
        },
      },
      roles: { primary: "general" },
    },
    mcp: { servers: {} },
    provider_authentication: { company: "personal" },
    mcp_authentication: {},
    credential_status: { providers: {}, mcp: {} },
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify(settings))),
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

it.each([false, true])(
  "shows the instance routing graph immediately in the editor: %s",
  async (editing) => {
    await act(async () => {
      root.render(
        <ChatSettings
          project="instance"
          identity="admin"
          canAdminister
          instanceScope
          section="routing"
          settingsPath="/api/admin/chat/settings"
          onClose={() => {}}
          onSaved={() => {}}
          onOpenEditor={() => {}}
          {...(editing ? { editorLocation: { action: "edit" as const } } : {})}
        />,
      );
      await import("@colossus/ui/components/ModelRoutingGraph");
    });
    expect(
      container.querySelector('[aria-label="Model routing graph"]'),
    ).not.toBeNull();
    expect(container.querySelector("summary")).toBeNull();
    expect(container.querySelector("details")).toBeNull();
    expect(container.textContent).toContain("general-model");
    expect(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Primary assignment"]',
      )?.disabled,
    ).toBe(!editing);
  },
);

it("keeps project routing collapsed until the user opens its map", async () => {
  await act(async () => {
    root.render(
      <ChatSettings
        project="p"
        identity="admin"
        canAdminister
        section="routing"
        onClose={() => {}}
        onSaved={() => {}}
        onOpenEditor={() => {}}
      />,
    );
  });
  const details = container.querySelector("details");
  expect(details?.open).toBe(false);
  expect(container.querySelector("summary")?.textContent).toBe("Routing map");
  expect(
    container.querySelector('[aria-label="Model routing graph"]'),
  ).toBeNull();
  await act(async () => {
    details!.open = true;
    details!.dispatchEvent(new Event("toggle"));
    await import("@colossus/ui/components/ModelRoutingGraph");
  });
  expect(
    container.querySelector('[aria-label="Model routing graph"]'),
  ).not.toBeNull();
});
