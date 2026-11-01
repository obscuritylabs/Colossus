// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatSettings } from "./ChatSettings";
import type { ChatSettingsSnapshot, ChatSettingsWrite } from "./chat-settings";
import type { SettingsEditorNavigation } from "./settings-editor";
import type { ConnectionSettingsSection } from "./routes";

let root: Root, container: HTMLDivElement;
let settings: ChatSettingsSnapshot;
let writes: { path: string; body: Record<string, unknown>; method: string }[];
let pendingSave: ((response: Response) => void) | null;
let pendingRead: ((response: Response) => void) | null;
let pendingCatalog: ((response: Response) => void) | null;
let delaySettingsRead: boolean;
let credentialRevision: number;
let providerPresets: unknown;
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
  providerPresets = { presets: [] };
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
        if (path.endsWith("/provider-presets")) return json(providerPresets);
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
          if (response.ok)
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
  section: ConnectionSettingsSection = "providers",
  editorNavigation: SettingsEditorNavigation & { instanceScope?: boolean } = {},
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
        {...editorNavigation}
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
            documentInputs: "off",
            declared: { toolCalls: true },
          },
        },
      },
    },
    mcp: { oauthCredentialStore: "encrypted_state" },
  });
  await act(() => pendingSave!(json({ revision: 8 })));
});

it.each(["on", "auto"] as const)(
  "preserves the explicit %s document preference and declaration in web model settings",
  async (mode) => {
    settings.models.profiles.careful!.capabilities = {
      ...settings.models.profiles.careful!.capabilities,
      documentInputs: mode,
      declared: { toolCalls: true, documentInputs: true },
    };
    await render(true, "admin", "models");
    await act(() => button("Edit careful").click());
    expect(
      container
        .querySelector('[aria-label="Document inputs"]')
        ?.getAttribute("aria-valuetext"),
    ).toBe(mode === "on" ? "On" : "Auto");
    expect(container.textContent).toContain(
      "web chat file picker currently accepts text files only",
    );
    await input("Maximum output (tokens)", "8192");
    await apply();
    expect(writes[0]!.body).toMatchObject({
      models: {
        profiles: {
          careful: {
            capabilities: {
              documentInputs: mode,
              declared: { documentInputs: true },
            },
          },
        },
      },
    });
    await act(() => pendingSave!(json({ revision: 8 })));
  },
);

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

it.each(["provider", "primary"] as const)(
  "explains an unavailable %s before model discovery while retaining definition edits",
  async (blocked) => {
    const reason = `${blocked} blocked by the project's Network access policy.`;
    settings.connection_availability = {
      providers: { company: { enabled: blocked !== "provider", reason } },
      models: {},
      mcp: {},
      search: {},
      roles: { primary: { enabled: blocked !== "primary", reason } },
    };
    await render(true, "admin", "models");
    await act(() => button("Edit careful").click());
    expect(button("Load models").disabled).toBe(true);
    expect(container.textContent).toContain(reason);
    expect(pendingCatalog).toBeNull();
    expect(writes).toHaveLength(0);
    await input("Model identifier", "configured-model-6.1");
    await apply();
    expect(writes).toHaveLength(1);
    expect(writes[0]!.body).toMatchObject({
      models: { profiles: { careful: { model: "configured-model-6.1" } } },
    });
    await act(() => pendingSave!(json({ revision: 8 })));
  },
);

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

it("keeps global catalog presets available independently of a project's origin policy", async () => {
  providerPresets = {
    presets: [
      {
        id: "openrouter",
        label: "OpenRouter",
        protocol: "chat_completions",
        baseUrl: "https://openrouter.ai/api/v1",
        credentialEnv: "OPENROUTER_API_KEY",
      },
    ],
    availability: {
      openrouter: {
        status: "available",
        reason: "Available catalog definition.",
      },
    },
  };
  await render(true, "admin", "providers", {
    instanceScope: true,
    editorLocation: { action: "new" },
  });
  await act(() => button("Provider").click());
  const openRouter = [
    ...document.querySelectorAll<HTMLButtonElement>('[role="option"]'),
  ].find((option) => option.textContent?.trim() === "OpenRouter");
  expect(openRouter).toBeDefined();
  await act(() => openRouter!.click());
  expect(field("Endpoint URL")?.value).toBe("https://openrouter.ai/api/v1");
  expect(button("Authentication").textContent).toContain(
    "Personal account (each user)",
  );
  expect(button("Add provider").disabled).toBe(false);
  expect(container.textContent).toContain("Available catalog definition.");
  expect(container.textContent).not.toContain(
    "At least one project's operator network policy does not permit this endpoint.",
  );
  expect(writes).toEqual([]);
});

it("opens a provider form from its complete location and returns only after revision readback", async () => {
  const onCloseEditor = vi.fn(),
    onDirtyChange = vi.fn();
  await render(true, "admin", "providers", {
    editorLocation: { action: "edit", id: "company" },
    onCloseEditor,
    onDirtyChange,
  });
  expect(container.querySelector(".provider-editor")).not.toBeNull();
  expect(container.querySelector("table")).toBeNull();
  expect(field("Endpoint URL")?.value).toBe("https://models.example/v1");
  expect(container.textContent).toContain("project override");
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  await input("Display label", "Company production");
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  await apply();
  expect(button("Save changes").disabled).toBe(true);
  delaySettingsRead = true;
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(onCloseEditor).not.toHaveBeenCalled();
  delaySettingsRead = false;
  await act(() => pendingRead!(json(settings)));
  expect(onCloseEditor).toHaveBeenCalledOnce();
  expect(onDirtyChange).toHaveBeenCalledWith(false);
  expect(writes[0]!.body).toMatchObject({
    labels: { providers: { company: "Company production" } },
  });
});

it("navigates to a new form without opening an inline editor", async () => {
  const onOpenEditor = vi.fn();
  await render(true, "admin", "models", { onOpenEditor });
  await act(() => button("Add model").click());
  expect(onOpenEditor).toHaveBeenCalledWith({ action: "new" });
  expect(container.querySelector(".model-editor")).toBeNull();
  await render(true, "admin", "models", {
    editorLocation: { action: "new" },
    onOpenEditor,
  });
  expect(container.querySelector(".model-editor")).not.toBeNull();
  expect(field("Profile ID")?.value).toBe("");
});

it("does not turn a missing direct edit into a new definition", async () => {
  const onCloseEditor = vi.fn();
  await render(true, "admin", "models", {
    editorLocation: { action: "edit", id: "removed-model" },
    onCloseEditor,
  });
  expect(container.textContent).toContain("Setting unavailable");
  expect(container.querySelector("form")).toBeNull();
  expect(writes).toEqual([]);
  await act(() => button("Back to Models").click());
  expect(onCloseEditor).toHaveBeenCalledOnce();
});

it("retains a routed provider draft after the server rejects the save", async () => {
  const onCloseEditor = vi.fn();
  await render(true, "admin", "providers", {
    editorLocation: { action: "edit", id: "company" },
    onCloseEditor,
  });
  await input("Display label", "Unsaved provider label");
  await apply();
  await act(() =>
    pendingSave!(
      new Response(
        JSON.stringify({
          message: "Configuration changed. Reload and review.",
        }),
        { status: 409 },
      ),
    ),
  );
  expect(container.querySelector(".provider-editor")).not.toBeNull();
  expect(field("Display label")?.value).toBe("Unsaved provider label");
  expect(container.querySelector('[role="alert"]')).not.toBeNull();
  expect(onCloseEditor).not.toHaveBeenCalled();
});

it("rejects unsupported or credentialed MCP URLs before submitting", async () => {
  await render(true, "admin", "mcp", {
    editorLocation: { action: "edit", id: "docs" },
  });
  await input("MCP server URL", "https://user:private@docs.example/mcp");
  await apply();
  expect(writes).toEqual([]);
  expect(container.textContent).toContain("without embedded credentials");
});

it("rejects non-positive model limits even when form submission is programmatic", async () => {
  await render(true, "admin", "models", {
    editorLocation: { action: "edit", id: "careful" },
  });
  await input("Maximum output (tokens)", "0");
  await apply();
  expect(writes).toEqual([]);
  expect(container.textContent).toContain("positive whole numbers");
});

it("rejects dotted model Profile IDs before writing while retaining the provider model identifier", async () => {
  await render(true, "admin", "models", {
    editorLocation: { action: "new" },
  });
  await input("Display label", "Reasoner");
  await input("Profile ID", "gpt-6.1-sol");
  await input("Model identifier", "vendor/gpt-6.1-sol");
  await apply();
  expect(writes).toEqual([]);
  expect(container.textContent).toContain(
    "Profile ID of 1–128 letters, digits, hyphens, or underscores",
  );
  expect(container.textContent).toContain(
    "Dots and slashes are allowed in Model identifier.",
  );
  expect(field("Model identifier")?.value).toBe("vendor/gpt-6.1-sol");
});

it.each([
  { model: "gpt-6.1-sol", profile: "gpt-6-1-sol" },
  { model: "vendor/gpt-6.1-sol", profile: "vendor-gpt-6-1-sol" },
])(
  "creates a cloud-safe catalog profile for $model without altering its model identifier",
  async ({ model, profile }) => {
    await render(true, "admin", "models", {
      editorLocation: { action: "new" },
    });
    await act(() => button("Load models").click());
    await act(() =>
      pendingCatalog!(
        json({
          models: [
            {
              id: model,
              display_name: "Catalog reasoner",
              context_window_tokens: 128000,
              max_output_tokens: 16000,
              document_inputs: true,
            },
          ],
        }),
      ),
    );
    await act(() =>
      container
        .querySelector<HTMLButtonElement>(".provider-model-card")!
        .click(),
    );
    expect(field("Profile ID")?.value).toBe(profile);
    expect(field("Model identifier")?.value).toBe(model);
    await apply();
    const updates = writes.filter((write) => write.method === "PUT");
    expect(updates).toHaveLength(1);
    expect(updates[0]!.body).toMatchObject({
      models: {
        profiles: {
          [profile]: {
            providerProfile: "company",
            model,
            capabilities: {
              documentInputs: "off",
              declared: { documentInputs: true },
            },
          },
        },
      },
    });
    await act(() => pendingSave!(json({ revision: 8 })));
  },
);

it("rejects maximum output equal to the model's context window before writing", async () => {
  await render(true, "admin", "models", {
    editorLocation: { action: "edit", id: "careful" },
  });
  await input("Context window (tokens)", "32768");
  await input("Maximum output (tokens)", "32768");
  await apply();
  expect(writes).toEqual([]);
  expect(container.textContent).toContain(
    "Maximum output must be smaller than the context window.",
  );
});

it.each([
  { context: 1024, output: 512, maximum: 511, reserve: "512" },
  { context: 32768, output: 29491, maximum: 29490, reserve: "3,277" },
])(
  "requires input space beyond the safety reserve for a $context token context",
  async ({ context, output, maximum, reserve }) => {
    await render(true, "admin", "models", {
      editorLocation: { action: "edit", id: "careful" },
    });
    await input("Context window (tokens)", String(context));
    await input("Maximum output (tokens)", String(output));
    await apply();
    expect(writes).toEqual([]);
    expect(container.textContent).toContain(`${reserve} token safety reserve`);
    await input("Maximum output (tokens)", String(maximum));
    await apply();
    expect(writes).toHaveLength(1);
    expect(writes[0]!.body).toMatchObject({
      models: {
        profiles: {
          careful: { contextWindowTokens: context, maxOutputTokens: maximum },
        },
      },
    });
    await act(() => pendingSave!(json({ revision: 8 })));
  },
);

it("rejects a context below the server minimum during programmatic submission", async () => {
  await render(true, "admin", "models", {
    editorLocation: { action: "edit", id: "careful" },
  });
  await input("Context window (tokens)", "1023");
  await input("Maximum output (tokens)", "128");
  await apply();
  expect(writes).toEqual([]);
  expect(container.textContent).toContain(
    "Context window must be at least 1,024 tokens.",
  );
});

it.each([
  { name: "more than 96 UTF-8 bytes", label: "é".repeat(49) },
  { name: "a control character", label: "Reasoner\u0007label" },
])(
  "rejects a model label containing $name before writing",
  async ({ label }) => {
    await render(true, "admin", "models", {
      editorLocation: { action: "edit", id: "careful" },
    });
    await input("Display label", label);
    await apply();
    expect(writes).toEqual([]);
    expect(container.textContent).toContain(
      "Display label must contain 1–96 UTF-8 bytes and no control characters.",
    );
  },
);

it.each([
  { name: "more than 1,024 UTF-8 bytes", model: "é".repeat(513) },
  { name: "a control character", model: "vendor/model\u0085suffix" },
])(
  "rejects a model identifier containing $name before writing",
  async ({ model }) => {
    await render(true, "admin", "models", {
      editorLocation: { action: "edit", id: "careful" },
    });
    await input("Model identifier", model);
    await apply();
    expect(writes).toEqual([]);
    expect(container.textContent).toContain(
      "Model identifier must contain 1–1,024 UTF-8 bytes and no control characters.",
    );
  },
);

it("accepts model text at the UTF-8 limits while preserving model identifier punctuation", async () => {
  await render(true, "admin", "models", {
    editorLocation: { action: "edit", id: "careful" },
  });
  const label = "é".repeat(48);
  const model = "vendor/gpt-6.1-" + "m".repeat(1009);
  expect(new TextEncoder().encode(model)).toHaveLength(1024);
  await input("Display label", label);
  await input("Model identifier", model);
  await apply();
  expect(writes).toHaveLength(1);
  expect(writes[0]!.body).toMatchObject({
    labels: { models: { careful: label } },
    models: { profiles: { careful: { model } } },
  });
  await act(() => pendingSave!(json({ revision: 8 })));
});

it("reconstructs direct search and variable editors with inherited scope metadata", async () => {
  settings.search = {
    profiles: {
      web: {
        kind: "searxng",
        endpoint: "https://search.example/search",
        credentialReference: null,
        authHeader: "X-Search-Key",
        userAgent: "colossus",
        timeoutMs: 30000,
      },
    },
    roles: { agent: "web" },
  };
  settings.search_authentication = { web: "none" };
  settings.variables = { REGION: "us-east" };
  await render(true, "admin", "search", {
    editorLocation: { action: "edit", id: "web" },
  });
  expect(container.querySelector(".service-settings-editor")).not.toBeNull();
  expect(field("Endpoint URL")?.value).toBe("https://search.example/search");
  expect(container.querySelector("table")).toBeNull();
  await render(true, "admin", "variables", {
    editorLocation: { action: "edit", id: "REGION" },
  });
  expect(field("Variable name")?.value).toBe("REGION");
  expect(field("Variable name")?.disabled).toBe(true);
  expect(field("Variable value")?.value).toBe("us-east");
  expect(container.querySelector("table")).toBeNull();
});

it("opens an instance collector page with restart and privacy guidance", async () => {
  await render(true, "admin", "telemetry", {
    instanceScope: true,
    editorLocation: { action: "edit" },
  });
  expect(container.querySelector(".service-settings-editor")).not.toBeNull();
  expect(container.textContent).toContain("restarting the Control Plane");
  expect(container.textContent).toContain(
    "Private conversation and tool payloads are excluded",
  );
  expect(container.querySelector('input[type="url"]')).not.toBeNull();
  expect(field("Display label")).toBeUndefined();
});

it("rejects collector editing in project scope", async () => {
  await render(true, "admin", "telemetry", {
    editorLocation: { action: "edit" },
  });
  expect(container.textContent).toContain("Setting unavailable");
  expect(container.querySelector("form")).toBeNull();
});

it("opens credentials for an unchanged saved provider without first cancelling its page", async () => {
  const onCloseEditor = vi.fn(),
    onDirtyChange = vi.fn();
  await render(true, "admin", "providers", {
    editorLocation: { action: "edit", id: "company" },
    onCloseEditor,
    onDirtyChange,
  });
  await act(() => button("Choose or add credential").click());
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(onCredentials).toHaveBeenCalledWith({
    kind: "provider",
    id: "company",
  });
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  expect(writes).toEqual([]);
});

it("shows inherited definitions without granting override or credential actions before hosted execution is enabled", async () => {
  settings.execution = {
    enabled: false,
    settings_editable: false,
    reason: "A deployment operator must configure hosted execution.",
  };
  const onOpenEditor = vi.fn(),
    onDirtyChange = vi.fn();
  await render(true, "admin", "providers", { onOpenEditor, onDirtyChange });
  expect(container.textContent).toContain(
    "A deployment operator must configure hosted execution.",
  );
  expect(button("Add provider").disabled).toBe(true);
  expect(button("Edit company").disabled).toBe(true);
  expect(button("Manage credentials").disabled).toBe(true);
  expect(button("Reload settings").disabled).toBe(false);
  expect(writes).toEqual([]);
  await render(true, "admin", "providers", {
    editorLocation: { action: "edit", id: "company" },
    onDirtyChange,
  });
  expect(container.querySelector("form")).toBeNull();
  expect(container.textContent).toContain("Setting unavailable");
  expect(container.textContent).toContain(
    "A deployment operator must configure hosted execution.",
  );
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
});

it("does not request a project credential store for a dormant project", async () => {
  settings.execution = {
    enabled: false,
    settings_editable: false,
    reason: "Hosted execution is not enabled.",
  };
  await render(true, "admin", "credentials");
  expect(container.textContent).toContain("Hosted execution is not enabled.");
  expect(container.querySelector("form")).toBeNull();
  expect(
    vi
      .mocked(fetch)
      .mock.calls.some(([path]) => String(path).includes("credential-store")),
  ).toBe(false);
  expect(writes).toEqual([]);
});

it("retains a dirty routed search form when its guarded Cancel navigation is rejected", async () => {
  settings.search = {
    profiles: {
      web: {
        kind: "searxng",
        endpoint: "https://search.example/search",
        credentialReference: null,
      },
    },
    roles: {},
  };
  const onCloseEditor = vi.fn(),
    onDirtyChange = vi.fn();
  await render(true, "admin", "search", {
    editorLocation: { action: "edit", id: "web" },
    onCloseEditor,
    onDirtyChange,
  });
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  await input("Endpoint URL", "https://search.example/new/search");
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  await act(() => button("Cancel").click());
  expect(onCloseEditor).toHaveBeenCalledOnce();
  expect(field("Endpoint URL")?.value).toBe(
    "https://search.example/new/search",
  );
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  expect(writes).toEqual([]);
});

it("admits one catalog write for duplicate submit events in the same turn", async () => {
  await render(true, "admin", "providers", {
    editorLocation: { action: "edit", id: "company" },
  });
  await input("Display label", "One immutable submission");
  await act(() => {
    const form = container.querySelector("form")!;
    for (let index = 0; index < 2; index++)
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
  });
  expect(writes).toHaveLength(1);
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(onSaved).toHaveBeenCalledOnce();
});

it("locks a routed variable proposal after conflict and reloads only after a confirmed current read", async () => {
  settings.variables = { REGION: "original" };
  const onCloseEditor = vi.fn(),
    onDirtyChange = vi.fn(),
    confirm = vi.fn(() => false);
  vi.stubGlobal("confirm", confirm);
  await render(true, "admin", "variables", {
    editorLocation: { action: "edit", id: "REGION" },
    onCloseEditor,
    onDirtyChange,
  });
  await input("Variable value", "proposed");
  await apply();
  await act(() =>
    pendingSave!(
      new Response(JSON.stringify({ error: "conflict" }), { status: 409 }),
    ),
  );
  expect(field("Variable value")?.value).toBe("proposed");
  expect(button("Save variable").disabled).toBe(true);
  expect(button("Reload settings").disabled).toBe(false);
  expect(onSaved).not.toHaveBeenCalled();
  expect(onCloseEditor).not.toHaveBeenCalled();
  const readsBefore = vi.mocked(fetch).mock.calls.length;
  await act(() => button("Reload settings").click());
  expect(vi.mocked(fetch).mock.calls.length).toBe(readsBefore);
  expect(field("Variable value")?.value).toBe("proposed");
  confirm.mockReturnValue(true);
  delaySettingsRead = true;
  await act(() => button("Reload settings").click());
  expect(field("Variable value")?.value).toBe("proposed");
  expect(button("Save variable").disabled).toBe(true);
  await act(() =>
    pendingRead!(
      new Response(JSON.stringify({ error: "storage" }), { status: 503 }),
    ),
  );
  expect(field("Variable value")?.value).toBe("proposed");
  expect(button("Save variable").disabled).toBe(true);
  settings = { ...settings, revision: 10, variables: { REGION: "current" } };
  await act(() => button("Reload settings").click());
  await act(() => pendingRead!(json(settings)));
  expect(field("Variable value")?.value).toBe("current");
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  await input("Variable value", "reviewed-update");
  await apply();
  expect(writes.at(-1)?.body.revision).toBe(10);
  delaySettingsRead = false;
  await act(() => pendingSave!(json({ revision: 11 })));
});
it("retains an accepted proposal and locks saving when canonical readback fails", async () => {
  settings.variables = { REGION: "original" };
  vi.stubGlobal(
    "confirm",
    vi.fn(() => true),
  );
  const onCloseEditor = vi.fn();
  await render(true, "admin", "variables", {
    editorLocation: { action: "edit", id: "REGION" },
    onCloseEditor,
  });
  await input("Variable value", "accepted-proposal");
  await apply();
  delaySettingsRead = true;
  await act(() => pendingSave!(json({ revision: 8 })));
  await act(() =>
    pendingRead!(
      new Response(JSON.stringify({ error: "storage" }), { status: 503 }),
    ),
  );
  expect(button("Save variable").disabled).toBe(true);
  expect(field("Variable value")?.value).toBe("accepted-proposal");
  expect(onSaved).not.toHaveBeenCalled();
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "accepted",
  );
  await act(() => button("Reload settings").click());
  await act(() =>
    pendingRead!(
      json({ ...settings, revision: 7, variables: { REGION: "stale-read" } }),
    ),
  );
  expect(field("Variable value")?.value).toBe("accepted-proposal");
  expect(button("Save variable").disabled).toBe(true);
  await act(() => button("Reload settings").click());
  await act(() => pendingRead!(json(settings)));
  expect(field("Variable value")?.value).toBe("accepted-proposal");
  await input("Variable value", "next-update");
  expect(button("Save variable").disabled).toBe(false);
});
it("reinitializes a routed provider only after an explicit reload even when its revision is unchanged", async () => {
  vi.stubGlobal(
    "confirm",
    vi.fn(() => true),
  );
  await render(true, "admin", "providers", {
    editorLocation: { action: "edit", id: "company" },
  });
  await input("Display label", "retained-provider-proposal");
  await apply();
  await act(() =>
    pendingSave!(
      new Response(JSON.stringify({ error: "storage" }), { status: 503 }),
    ),
  );
  expect(button("Save changes").disabled).toBe(true);
  settings = {
    ...settings,
    revision: 7,
    labels: {
      providers: { company: "Reviewed original" },
      models: {},
      search: {},
    },
  };
  await act(() => button("Reload settings").click());
  expect(field("Display label")?.value).toBe("Reviewed original");
  expect(button("Save changes").disabled).toBe(false);
});

it("updates runtime availability from an explicitly reloaded settings snapshot", async () => {
  settings.execution = {
    enabled: false,
    settings_editable: false,
    reason: "Waiting for an operator.",
  };
  await render();
  expect(button("Add provider").disabled).toBe(true);
  settings = {
    ...settings,
    revision: 8,
    execution: { enabled: true, settings_editable: true },
  };
  await act(() => button("Reload settings").click());
  expect(button("Add provider").disabled).toBe(false);
  expect(writes).toEqual([]);
});

it("allows correcting a new provider after a definitive rejected save without staging its profile", async () => {
  await render(true, "admin", "providers", {
    editorLocation: { action: "new" },
  });
  await input("Display label", "New provider");
  await input("Profile ID", "new-provider");
  await input("Endpoint URL", "https://unreviewed.example/v1");
  await apply();
  await act(() =>
    pendingSave!(
      new Response(JSON.stringify({ error: "invalid_argument" }), {
        status: 400,
      }),
    ),
  );
  expect(field("Profile ID")?.value).toBe("new-provider");
  expect(button("Add provider").disabled).toBe(false);
  expect(onSaved).not.toHaveBeenCalled();
  await input("Endpoint URL", "https://models.example/v1");
  await apply();
  expect(writes).toHaveLength(2);
  expect(writes[0]!.body.revision).toBe(7);
  expect(writes[1]!.body).toMatchObject({
    revision: 7,
    providers: {
      profiles: { "new-provider": { baseUrl: "https://models.example/v1" } },
    },
  });
  expect(container.textContent).not.toContain("already exists");
  await act(() => pendingSave!(json({ revision: 8 })));
  expect(onSaved).toHaveBeenCalledOnce();
});
