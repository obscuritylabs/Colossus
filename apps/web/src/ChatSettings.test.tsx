// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatSettings } from "./ChatSettings";
import type { ChatSettingsSnapshot, ChatSettingsWrite } from "./chat-settings";

let root: Root, container: HTMLDivElement;
let settings: ChatSettingsSnapshot;
let writes: { path: string; body: Record<string, unknown>; method: string }[];
let pendingSave: ((response: Response) => void) | null;
let pendingRead: ((response: Response) => void) | null;
let pendingCatalog: ((response: Response) => void) | null;
let delaySettingsRead: boolean;
let credentialRevision: number;
const onCredentials = vi.fn(),
  onPersonalCredentials = vi.fn();
const onSaved = vi.fn();
const json = (value: unknown) => new Response(JSON.stringify(value));
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  localStorage.clear();
  sessionStorage.clear();
  onSaved.mockReset();
  onCredentials.mockReset();
  onPersonalCredentials.mockReset();
  credentialRevision = 7;
  pendingSave = null;
  pendingRead = null;
  pendingCatalog = null;
  delaySettingsRead = false;
  settings = {
    revision: 7,
    providers: {
      profiles: {
        company: {
          kind: "openai_responses",
          baseUrl: "https://models.example/v1",
          credentialReference: null,
          timeoutMs: 300000,
          generationTimeoutMs: 1200000,
        },
      },
    },
    models: {
      profiles: {
        careful: {
          providerProfile: "company",
          model: "reasoning-model",
          contextWindowTokens: 128000,
          maxOutputTokens: 16000,
          reasoningEffort: "high",
          capabilities: {
            toolCalls: "on",
            streaming: "on",
            imageInputs: "off",
            serverCompaction: "auto",
            declared: { toolCalls: true },
          },
        },
      },
      roles: { primary: "careful", planner: "careful" },
    },
    mcp: {
      oauthCredentialStore: "encrypted_state",
      servers: {
        docs: {
          transport: "streamable_http",
          url: "https://docs.example/mcp",
          allowedTools: ["search", "read"],
          allowStateless: true,
          protocolVersion: "2025-11-25",
          oauth: {
            clientId: "registered-client",
            callbackPort: 8787,
            scopes: ["read"],
          },
        },
      },
    },
    provider_authentication: { company: "shared" },
    mcp_authentication: { docs: "personal" },
    credential_status: { providers: { company: true }, mcp: {} },
  };
  writes = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit) => {
      if (options.method === "GET") {
        if (path.endsWith("/credential-store/project"))
          return json({
            revision: credentialRevision,
            credentials: [],
            connections: [],
          });
        if (path.endsWith("/provider-presets")) return json({ presets: [] });
        if (delaySettingsRead)
          return new Promise<Response>((resolve) => {
            pendingRead = resolve;
          });
        return json(settings);
      }
      const body = JSON.parse(String(options.body));
      writes.push({ path, body, method: options.method! });
      if (path.endsWith("/credential-store/project")) {
        credentialRevision++;
        return json({ revision: credentialRevision });
      }
      if (path.endsWith("/models"))
        return new Promise<Response>((resolve) => {
          pendingCatalog = resolve;
        });
      if (path.endsWith("/credential")) {
        settings.revision++;
        return json({ revision: settings.revision });
      }
      return new Promise<Response>((resolve) => {
        pendingSave = (response) => {
          const { inheritance, ...configuration } =
            body as unknown as ChatSettingsWrite;
          settings = {
            ...settings,
            ...configuration,
            ...(inheritance && settings.inheritance
              ? {
                  inheritance: {
                    ...settings.inheritance,
                    global_revision: inheritance.global_revision,
                  },
                }
              : {}),
            revision: settings.revision + 1,
          };
          resolve(response);
        };
      });
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(
  canAdminister = true,
  identity = "admin",
  section:
    "providers" | "models" | "mcp" | "credentials" | "routing" = "providers",
) {
  await act(async () =>
    root.render(
      <ChatSettings
        project="p"
        identity={identity}
        section={section}
        canAdminister={canAdminister}
        onClose={() => {}}
        onSaved={onSaved}
        onCredentials={onCredentials}
        onPersonalCredentials={onPersonalCredentials}
      />,
    ),
  );
}
function button(text: string) {
  const item = [
    ...container.querySelectorAll<HTMLButtonElement>("button"),
  ].find(
    (value) =>
      value.textContent?.trim() === text ||
      value.getAttribute("aria-label") === text,
  );
  if (!item) throw new Error(`Missing button ${text}`);
  return item;
}
function field(label: string) {
  return (
    container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`) ??
    [...container.querySelectorAll("label")]
      .find((element) => element.querySelector("span")?.textContent === label)
      ?.querySelector<HTMLInputElement>("input")
  );
}
async function input(label: string, value: string) {
  const item = field(label);
  if (!item) throw new Error(`Missing input ${label}`);
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(item, value);
    item.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function apply() {
  await act(() =>
    container
      .querySelector<HTMLFormElement>(
        "form.provider-editor, form.model-editor, form.chat-settings-editor",
      )!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}

it("edits typed provider settings while preserving advanced fields and submitting one immutable revision", async () => {
  await render();
  await act(() => button("Edit company").click());
  await input("Request timeout (ms)", "450000");
  await apply();
  expect(writes).toHaveLength(1);
  expect(writes[0]).toMatchObject({
    method: "PUT",
    body: {
      revision: 7,
      providers: {
        profiles: {
          company: { timeoutMs: 450000, generationTimeoutMs: 1200000 },
        },
      },
    },
  });
  expect(writes[0]!.body).not.toHaveProperty("credential_status");
  expect(button("Saving…").disabled).toBe(true);
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(container.textContent).toContain("Project configuration saved");
  expect(onSaved).toHaveBeenCalledOnce();
});

it("preserves model feature declarations and role routing when editing model limits", async () => {
  await render(true, "admin", "models");
  await act(() => button("Edit careful").click());
  await input("Maximum output (tokens)", "8192");
  await apply();
  expect(writes[0]!.body).toMatchObject({
    models: {
      roles: { primary: "careful", planner: "careful" },
      profiles: {
        careful: {
          maxOutputTokens: 8192,
          capabilities: {
            serverCompaction: "auto",
            declared: { toolCalls: true },
          },
        },
      },
    },
    mcp: { oauthCredentialStore: "encrypted_state" },
  });
  await act(() => pendingSave!(json({ revision: 8 })));
});

it("locks new edits until the saved revision is read back and then saves against that revision", async () => {
  await render();
  await act(() => button("Edit company").click());
  await input("Request timeout (ms)", "450000");
  await apply();
  delaySettingsRead = true;
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(pendingRead).not.toBeNull();
  expect(button("Add provider").disabled).toBe(true);
  expect(button("Edit company").disabled).toBe(true);
  expect(writes).toHaveLength(1);
  delaySettingsRead = false;
  await act(() => pendingRead!(json(settings)));
  expect(button("Add provider").disabled).toBe(false);
  expect(button("Edit company").disabled).toBe(false);
  await act(() => button("Edit company").click());
  expect(field("Request timeout (ms)")?.value).toBe("450000");
  await input("Request timeout (ms)", "240000");
  await apply();
  expect(writes).toHaveLength(2);
  expect(writes[0]!.body).toMatchObject({
    revision: 7,
    providers: { profiles: { company: { timeoutMs: 450000 } } },
  });
  expect(writes[1]!.body).toMatchObject({
    revision: 8,
    providers: { profiles: { company: { timeoutMs: 240000 } } },
  });
  await act(() => pendingSave!(json({ revision: 9 })));
});

it("locks conflicting model mutations while discovery is pending and retains the draft across sections", async () => {
  await render(true, "admin", "models");
  await act(() => button("Edit careful").click());
  await input("Maximum output (tokens)", "8192");
  await act(() => button("Load models").click());
  expect(pendingCatalog).not.toBeNull();
  expect(button("Save changes").disabled).toBe(true);
  expect(button("Cancel").disabled).toBe(true);
  expect(field("Model identifier")?.disabled).toBe(true);
  expect(
    container.querySelector<HTMLButtonElement>(
      '.model-editor [role="combobox"]',
    )?.disabled,
  ).toBe(true);
  expect(button("Reload settings").disabled).toBe(true);
  await apply();
  expect(writes.filter((write) => write.method === "PUT")).toHaveLength(0);
  await render(true, "admin", "providers");
  expect(button("Add provider").disabled).toBe(true);
  expect(button("Cancel editing").disabled).toBe(true);
  await render(true, "admin", "models");
  expect(button("Save changes").disabled).toBe(true);
  expect(field("Maximum output (tokens)")?.value).toBe("8192");
  await act(() =>
    pendingCatalog!(
      json({
        models: [
          {
            id: "catalog-stable",
            display_name: "Stable synthetic model",
            context_window_tokens: 64000,
            max_output_tokens: 4000,
          },
        ],
      }),
    ),
  );
  expect(button("Save changes").disabled).toBe(false);
  expect(button("Cancel").disabled).toBe(false);
  expect(field("Model identifier")?.disabled).toBe(false);
  expect(field("Maximum output (tokens)")?.value).toBe("8192");
  const cards = [
    ...container.querySelectorAll<HTMLButtonElement>(".provider-model-card"),
  ];
  expect(cards).toHaveLength(1);
  expect(cards[0]!.textContent).toContain("Stable synthetic model");
  expect(writes).toEqual([
    {
      path: "/api/projects/p/chat/settings/providers/company/models",
      method: "POST",
      body: { revision: 7 },
    },
  ]);
});

it("edits remote MCP scopes and exact tools without losing protocol settings", async () => {
  await render(true, "admin", "mcp");
  await act(() => button("Edit docs").click());
  await input("Allowed tools", "search, read_document");
  await input("OAuth scopes", "read, profile");
  await apply();
  expect(writes[0]!.body).toMatchObject({
    mcp: {
      servers: {
        docs: {
          transport: "streamable_http",
          allowedTools: ["search", "read_document"],
          allowStateless: true,
          protocolVersion: "2025-11-25",
          oauth: { clientId: "registered-client", scopes: ["read", "profile"] },
        },
      },
    },
  });
  expect(JSON.stringify(writes[0]!.body)).not.toContain("secret");
  await act(() => pendingSave!(json({ revision: 8 })));
});

it("preserves an unsaved model across settings sections and cancels it without submitting", async () => {
  await render();
  await act(() => button("Edit company").click());
  await input("Request timeout (ms)", "450000");
  await apply();
  await act(() => pendingSave!(json({ revision: 8 })));
  await render(true, "admin", "models");
  await act(() => button("Edit careful").click());
  await input("Maximum output (tokens)", "8192");
  await render(true, "admin", "mcp");
  expect(container.textContent).toContain("An editor is still open in Models");
  await render(true, "admin", "models");
  expect(field("Maximum output (tokens)")?.value).toBe("8192");
  await act(() => button("Cancel").click());
  expect(writes).toHaveLength(1);
  expect(settings).toMatchObject({
    providers: { profiles: { company: { timeoutMs: 450000 } } },
    models: { profiles: { careful: { maxOutputTokens: 16000 } } },
  });
});

it("writes shared credentials separately and clears the input without storing the secret in configuration or browser storage", async () => {
  await render(true, "admin", "credentials");
  await act(() => button("Add credential").click());
  await input("Display label", "Project key");
  const field = container.querySelector<HTMLInputElement>(
    'input[aria-label="API key or token"]',
  )!;
  field.value = "synthetic-private-key";
  await act(() =>
    field
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(field.value).toBe("");
  expect(writes).toEqual([
    {
      path: "/api/projects/p/chat/credential-store/project",
      method: "POST",
      body: {
        revision: 7,
        label: "Project key",
        kind: "api_key",
        secret: "synthetic-private-key",
      },
    },
  ]);
  expect(container.textContent).not.toContain("synthetic-private-key");
  expect(container.innerHTML).not.toContain("synthetic-private-key");
  expect(JSON.stringify(settings)).not.toContain("synthetic-private-key");
  expect(localStorage.length).toBe(0);
  expect(sessionStorage.length).toBe(0);
});

it("does not fetch settings for read-only callers and ignores completion after authority changes", async () => {
  await render(false);
  expect(fetch).not.toHaveBeenCalled();
  expect(container.textContent).toContain("administration access is required");
  await render(true, "admin", "models");
  await act(() => button("Edit careful").click());
  await input("Model identifier", "updated-model");
  await apply();
  await render(false, "different-user");
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(onSaved).not.toHaveBeenCalled();
  expect(container.textContent).not.toContain("updated-model");
});

it("saves a new connection and reconciles its revision before opening credential selection", async () => {
  await render();
  await act(() => button("Add provider").click());
  await input("Display label", "New API");
  await input("Profile ID", "new-api");
  await input("Endpoint URL", "https://new.example/v1");
  expect(button("Save and choose credential").disabled).toBe(false);
  await act(() => button("Save and choose credential").click());
  expect(writes).toHaveLength(1);
  expect(writes[0]!.method).toBe("PUT");
  expect(onPersonalCredentials).not.toHaveBeenCalled();
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(onPersonalCredentials).toHaveBeenCalledWith({
    kind: "provider",
    id: "new-api",
  });
  expect(JSON.stringify(writes[0]!.body)).not.toContain("secret");
});
