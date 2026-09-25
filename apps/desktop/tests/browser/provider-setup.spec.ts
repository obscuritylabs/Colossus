import { expect, test, type Page } from "@playwright/test";

async function mountSetup(
  page: Page,
  configured = false,
  workspaceSelected = true,
  hasCredential = true,
) {
  await page.setViewportSize({ width: 1100, height: 1000 });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
  ).toBeVisible();
  await page.evaluate(
    async ({ configured, workspaceSelected, hasCredential }) => {
      const state = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        setupRequests: { command: string; args: Record<string, unknown> }[];
        releaseSlowCatalog: (() => void) | undefined;
        failCatalog: boolean;
        failPresets: boolean;
        failSave: boolean;
      };
      state.setupRequests = [];
      state.failCatalog = false;
      state.failPresets = false;
      state.failSave = false;
      state.__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: Record<string, unknown>) => {
          state.setupRequests.push({ command, args });
          if (command === "get_provider_presets") {
            if (state.failPresets)
              throw {
                code: "offline",
                message: "Presets unavailable.",
                retryable: true,
                outcomeUnknown: false,
                violations: [],
              };
            return [
              {
                id: "codex",
                label: "Codex (ChatGPT subscription)",
                protocol: "codex",
                baseUrl: "https://chatgpt.com/backend-api/codex",
                credentialEnv: null,
              },
              {
                id: "openrouter",
                label: "OpenRouter",
                protocol: "chat_completions",
                baseUrl: "https://openrouter.ai/api/v1",
                credentialEnv: "OPENROUTER_API_KEY",
              },
              {
                id: "custom-chat",
                label: "Custom Chat Completions",
                protocol: "chat_completions",
                baseUrl: null,
                credentialEnv: null,
              },
              {
                id: "custom-responses",
                label: "Custom Responses",
                protocol: "responses",
                baseUrl: null,
                credentialEnv: null,
              },
            ];
          }
          if (command === "discover_managed_provider_models") {
            const request = args.request as {
              baseUrl: string;
              credentialAction: string;
            };
            if (request.baseUrl.includes("slow")) {
              await new Promise<void>((resolve) => {
                state.releaseSlowCatalog = resolve;
              });
              return {
                credentialId: "old-credential",
                models: [{ id: "stale-model" }],
              };
            }
            if (state.failCatalog)
              return {
                credentialId: "saved-credential",
                models: [],
                errorMessage: "Authentication failed.",
              };
            return {
              credentialId:
                request.credentialAction === "none" ? null : "saved-credential",
              models: [
                {
                  id: "vendor/reasoner",
                  display_name: "Compact Reasoner",
                  description: "A model with catalog metadata.",
                  context_window_tokens: 8192,
                  max_output_tokens: 8192,
                  tool_calls: true,
                  image_inputs: false,
                },
                { id: "vendor/plain" },
              ],
            };
          }
          if (
            command === "configure_managed_runtime" ||
            command === "apply_managed_model_configuration"
          ) {
            if (state.failSave)
              throw {
                code: "invalid_configuration",
                message: "The model configuration could not be applied.",
                retryable: true,
                outcomeUnknown: false,
                violations: [],
              };
            return {};
          }
          return null;
        },
      };
      const modulePath = "/src/dev/provider-setup-studio.tsx";
      const fixture = await import(/* @vite-ignore */ modulePath);
      fixture.mountProviderSetupStudio(
        configured,
        workspaceSelected,
        hasCredential,
      );
    },
    { configured, workspaceSelected, hasCredential },
  );
  if (workspaceSelected) {
    await expect(
      page.getByRole("combobox", { name: "Provider", exact: true }),
    ).toContainText("OpenRouter");
  }
}

async function choosePreset(page: Page, name: string) {
  await page.getByRole("combobox", { name: "Provider", exact: true }).click();
  await page.getByRole("option", { name, exact: true }).click();
}

test("custom Responses setup searches model cards, imports metadata and saves native references", async ({
  page,
}) => {
  await mountSetup(page);
  await choosePreset(page, "Custom Responses");
  await page
    .getByLabel("API base URL", { exact: true })
    .fill("https://custom.example.test/v1");
  await page.getByLabel("Model ID", { exact: true }).fill("vendor/reasoner");
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("searchbox", { name: "Search models" }).fill("reasoner");
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  await expect(page.getByLabel("Model ID", { exact: true })).toHaveValue(
    "vendor/reasoner",
  );
  await page
    .getByText("Model limits and capabilities", { exact: true })
    .click();
  await expect(page.getByLabel("Context window (tokens)")).toHaveValue("8192");
  await expect(page.getByLabel("Maximum output (tokens)")).toHaveValue("4096");
  await page.getByLabel("Maximum output (tokens)").fill("2048");
  await page.getByRole("button", { name: "Refresh models" }).click();
  await expect(page.getByLabel("Maximum output (tokens)")).toHaveValue("2048");
  await page
    .getByRole("button", { name: "Toggle unrelated busy state" })
    .click();
  await page
    .getByRole("button", { name: "Toggle unrelated busy state" })
    .click();
  await expect(
    page.getByRole("button", { name: /Compact Reasoner/ }),
  ).toBeVisible();
  await page
    .locator(".provider-model-picker")
    .screenshot({ path: "output/playwright/provider-setup.png" });
  await page.getByRole("button", { name: "Save and start" }).click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: { command: string; args: { request: unknown } }[];
        }
      ).setupRequests.find(
        (entry) => entry.command === "configure_managed_runtime",
      )?.args.request,
  );
  expect(request).toMatchObject({
    providerKind: "openai_responses",
    baseUrl: "https://custom.example.test/v1",
    model: "vendor/reasoner",
    credentialId: "saved-credential",
    modelMetadata: {
      contextWindowTokens: 8192,
      maxOutputTokens: 2048,
      toolCalls: true,
      imageInputs: false,
      streaming: false,
    },
  });
});

test("catalog loading locks conflicting actions and workspace changes ignore pending results", async ({
  page,
}) => {
  await mountSetup(page);
  await choosePreset(page, "Custom Chat Completions");
  await page
    .getByLabel("API base URL", { exact: true })
    .fill("https://slow.example.test/v1");
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Loading models…" }),
  ).toBeVisible();
  await expect(page.getByLabel("API base URL", { exact: true })).toBeDisabled();
  for (const name of [
    "Choose another folder",
    "Save and start",
    "Run check",
    "Import connection",
  ]) {
    await expect(
      page.getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  }
  await page.getByRole("button", { name: "Switch workspace fixture" }).click();
  await page.evaluate(() =>
    (
      window as unknown as { releaseSlowCatalog: () => void }
    ).releaseSlowCatalog(),
  );
  await expect(page.getByRole("button", { name: /stale-model/ })).toHaveCount(
    0,
  );
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    "https://openrouter.ai/api/v1",
  );
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  await page
    .getByLabel("Model ID", { exact: true })
    .fill("private-manual-model");
  await page
    .getByText("Model limits and capabilities", { exact: true })
    .click();
  await expect(page.getByLabel("Context window (tokens)")).toHaveValue("32768");
  await expect(page.getByLabel("Tools", { exact: true })).not.toBeChecked();
  await expect(
    page.getByText(/This model’s details have not been loaded/),
  ).toBeVisible();
  await page
    .getByLabel("API base URL", { exact: true })
    .fill("https://slow.example.test/v1");
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: "Close setup fixture" }).click();
  await page.evaluate(() =>
    (
      window as unknown as { releaseSlowCatalog: () => void }
    ).releaseSlowCatalog(),
  );
  await page.getByRole("button", { name: "Open setup fixture" }).click();
  await expect(page.getByRole("button", { name: /stale-model/ })).toHaveCount(
    0,
  );
});

test("catalog failure retains enrolled credentials for retry and Codex uses its fixed endpoint", async ({
  page,
}) => {
  await mountSetup(page);
  await page.evaluate(() => {
    (window as unknown as { failCatalog: boolean }).failCatalog = true;
  });
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await expect(
    page.locator(".provider-model-picker").getByRole("alert"),
  ).toContainText("Authentication failed");
  await page.evaluate(() => {
    (window as unknown as { failCatalog: boolean }).failCatalog = false;
  });
  await page.getByRole("button", { name: "Retry loading models" }).click();
  await expect(
    page.getByRole("button", { name: /Compact Reasoner/ }),
  ).toBeVisible();
  const calls = await page.evaluate(() =>
    (
      window as unknown as {
        setupRequests: { command: string; args: { request: unknown } }[];
      }
    ).setupRequests
      .filter((entry) => entry.command === "discover_managed_provider_models")
      .map((entry) => entry.args.request),
  );
  expect(calls[1]).toMatchObject({
    credentialAction: "reuse",
    credentialId: "saved-credential",
  });
  await choosePreset(page, "Codex (ChatGPT subscription)");
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    "https://chatgpt.com/backend-api/codex",
  );
  await expect(page.getByLabel("API base URL", { exact: true })).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Load models", exact: true }),
  ).toBeEnabled();
});

test("retrying provider presets preserves an endpoint entered while loading failed", async ({
  page,
}) => {
  await mountSetup(page);
  await page.getByRole("button", { name: "Close setup fixture" }).click();
  await page.evaluate(() => {
    (window as unknown as { failPresets: boolean }).failPresets = true;
  });
  await page.getByRole("button", { name: "Open setup fixture" }).click();
  await expect(
    page.getByRole("button", { name: "Retry loading providers" }),
  ).toBeVisible();
  await page
    .getByLabel("API base URL", { exact: true })
    .fill("https://my-custom.example.test/v1");
  await page.evaluate(() => {
    (window as unknown as { failPresets: boolean }).failPresets = false;
  });
  await page.getByRole("button", { name: "Retry loading providers" }).click();
  await expect(
    page.getByRole("button", { name: "Retry loading providers" }),
  ).toHaveCount(0);
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    "https://my-custom.example.test/v1",
  );
});

test("replacing a stored key enrolls once and saves the newly discovered credential", async ({
  page,
}) => {
  await mountSetup(page, true);
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /Compact Reasoner/ }),
  ).toBeVisible();
  await page.getByLabel(/Use a different API key/).check();
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  const discoveryRequests = await page.evaluate(() =>
    (
      window as unknown as {
        setupRequests: { command: string; args: { request: unknown } }[];
      }
    ).setupRequests
      .filter((entry) => entry.command === "discover_managed_provider_models")
      .map((entry) => entry.args.request),
  );
  expect(discoveryRequests).toHaveLength(2);
  expect(discoveryRequests[0]).toMatchObject({ credentialAction: "reuse" });
  expect(discoveryRequests[1]).toMatchObject({ credentialAction: "replace" });
  expect(discoveryRequests[1]).not.toHaveProperty("credentialId");
  await page.getByRole("button", { name: "Save and start" }).click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: { command: string; args: { request: unknown } }[];
        }
      ).setupRequests.find(
        (entry) => entry.command === "configure_managed_runtime",
      )?.args.request,
  );
  expect(request).toMatchObject({
    credentialId: "saved-credential",
    replaceCredential: false,
    model: "vendor/reasoner",
  });
});

test("Codex authentication precedes its catalog and enables model loading after sign-in", async ({
  page,
}) => {
  await mountSetup(page);
  await choosePreset(page, "Codex (ChatGPT subscription)");
  await page.getByRole("button", { name: "Sign out", exact: true }).click();
  const signIn = page.getByRole("button", {
    name: "Sign in with ChatGPT",
    exact: true,
  });
  await expect(signIn).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Load models", exact: true }),
  ).toBeDisabled();
  expect(
    await signIn.evaluate((button) =>
      Boolean(
        button.compareDocumentPosition(
          document.querySelector(".provider-model-picker")!,
        ) & Node.DOCUMENT_POSITION_FOLLOWING,
      ),
    ),
  ).toBe(true);
  await signIn.click();
  await expect(
    page.getByRole("button", { name: "Load models", exact: true }),
  ).toBeEnabled();
});

for (const action of ["none", "replace"] as const) {
  test(`advanced setup clears a discovered credential when choosing ${action}`, async ({
    page,
  }) => {
    await mountSetup(page, true);
    await page
      .getByRole("button", {
        name: "Advanced model setup",
      })
      .click();
    await page
      .getByRole("button", { name: "Load models", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: /Compact Reasoner/ }),
    ).toBeVisible();
    await page.getByRole("combobox", { name: "API key", exact: true }).click();
    await page
      .getByRole("option", {
        name:
          action === "none"
            ? "No API key required"
            : "Enter or replace API key",
        exact: true,
      })
      .click();
    await page
      .getByRole("button", { name: "Load models", exact: true })
      .click();
    await page.getByRole("button", { name: /Compact Reasoner/ }).click();
    const requests = await page.evaluate(() =>
      (
        window as unknown as {
          setupRequests: { command: string; args: { request: unknown } }[];
        }
      ).setupRequests
        .filter((entry) => entry.command === "discover_managed_provider_models")
        .map((entry) => entry.args.request),
    );
    expect(requests[1]).toMatchObject({
      credentialAction: action,
      providerProfile: "primary-provider",
    });
    expect(requests[1]).not.toHaveProperty("credentialId");
    await page
      .getByRole("button", { name: "Save and start", exact: true })
      .click();
    await expect(
      page.getByText("Configuration saved", { exact: true }),
    ).toBeVisible();
    const request = await page.evaluate(
      () =>
        (
          window as unknown as {
            setupRequests: {
              command: string;
              args: { request: { providers: unknown[] } };
            }[];
          }
        ).setupRequests.find(
          (entry) => entry.command === "apply_managed_model_configuration",
        )?.args.request,
    );
    expect(request?.providers[0]).toMatchObject({
      credentialAction: action === "none" ? "none" : "reuse",
    });
    if (action === "none")
      expect(request?.providers[0]).not.toHaveProperty("credentialId");
    else
      expect(request?.providers[0]).toHaveProperty(
        "credentialId",
        "saved-credential",
      );
  });
}

test("folder selection loads native security choices and changing workspace clears the draft credential", async ({
  page,
}) => {
  await mountSetup(page, false, false);
  await expect(
    page.getByRole("combobox", { name: "Tool access", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Choose folder", exact: true })
    .click();
  await expect(
    page.getByRole("combobox", { name: "Tool access", exact: true }),
  ).toContainText("Allow all");
  await expect(
    page.getByRole("combobox", { name: "Command isolation", exact: true }),
  ).toContainText("Full access");
  await choosePreset(page, "Custom Responses");
  await page
    .getByLabel("API base URL", { exact: true })
    .fill("https://private.example.test/v1");
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  await page
    .getByRole("combobox", { name: "Tool access", exact: true })
    .click();
  await page
    .getByRole("option", {
      name: "Development — tools with approval checks",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "Toggle unrelated busy state" })
    .click();
  await page
    .getByRole("button", { name: "Toggle unrelated busy state" })
    .click();
  await expect(
    page.getByRole("combobox", { name: "Tool access", exact: true }),
  ).toContainText("Development");
  await page
    .getByRole("button", { name: "Choose another folder", exact: true })
    .click();
  await expect(page.getByLabel("Model ID", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    "https://openrouter.ai/api/v1",
  );
  await expect(
    page.getByRole("combobox", { name: "Tool access", exact: true }),
  ).toContainText("Allow all");
  await page.getByLabel("Model ID", { exact: true }).fill("fresh-model");
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: { command: string; args: { request: unknown } }[];
        }
      ).setupRequests.find(
        (entry) => entry.command === "configure_managed_runtime",
      )?.args.request,
  );
  expect(request).toMatchObject({
    workspaceId: "019b98f6-fd27-7413-bbe4-a44d97c0ff69",
    model: "fresh-model",
    accessProfile: "allow_all",
    executionBoundary: "full_access",
  });
  expect(request).not.toHaveProperty("credentialId");
});

test("simple and advanced setup preserve endpoint, model metadata and enrolled credentials", async ({
  page,
}) => {
  await mountSetup(page);
  await choosePreset(page, "Custom Responses");
  await page
    .getByLabel("API base URL", { exact: true })
    .fill("https://custom.example.test/v1");
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  await page
    .getByRole("button", {
      name: "Advanced model setup",
    })
    .click();
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    "https://custom.example.test/v1",
  );
  await expect(
    page.getByLabel("Provider model ID", { exact: true }),
  ).toHaveValue("vendor/reasoner");
  await expect(
    page.getByLabel("Context window (tokens)", { exact: true }),
  ).toHaveValue("8192");
  await expect(
    page.getByRole("combobox", { name: "API key", exact: true }),
  ).toContainText("Use saved API key");
  await page
    .getByLabel("Maximum output (tokens)", { exact: true })
    .fill("2048");
  await page
    .getByRole("button", { name: "Back to basic setup", exact: true })
    .click();
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveValue(
    "https://custom.example.test/v1",
  );
  await page
    .getByText("Model limits and capabilities", { exact: true })
    .click();
  await expect(page.getByLabel("Maximum output (tokens)")).toHaveValue("2048");
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: { command: string; args: { request: unknown } }[];
        }
      ).setupRequests.find(
        (entry) => entry.command === "configure_managed_runtime",
      )?.args.request,
  );
  expect(request).toMatchObject({
    baseUrl: "https://custom.example.test/v1",
    credentialId: "saved-credential",
    providerKind: "openai_responses",
    modelMetadata: {
      contextWindowTokens: 8192,
      maxOutputTokens: 2048,
      toolCalls: true,
    },
  });
});

test("existing multiple providers keep their routes and advanced save failures remain visible and retryable", async ({
  page,
}) => {
  await mountSetup(page);
  await page
    .getByRole("button", {
      name: "Load saved multiple providers fixture",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Back to basic setup", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Choose another folder", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByLabel("API base URL", { exact: true })).toHaveCount(2);
  await page.evaluate(() => {
    (window as unknown as { failSave: boolean }).failSave = true;
  });
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "The model configuration could not be applied." }),
  ).toBeVisible();
  await expect(
    page.getByLabel("Provider model ID", { exact: true }).nth(1),
  ).toHaveValue("vendor/secondary");
  await page.evaluate(() => {
    (window as unknown as { failSave: boolean }).failSave = false;
  });
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: {
            command: string;
            args: {
              request: {
                providers: unknown[];
                models: unknown[];
                roles: Record<string, string>;
              };
            };
          }[];
        }
      ).setupRequests
        .filter(
          (entry) => entry.command === "apply_managed_model_configuration",
        )
        .at(-1)?.args.request,
  );
  expect(request?.providers).toHaveLength(2);
  expect(request?.models).toHaveLength(2);
  expect(request?.models[1]).toMatchObject({
    reasoningEffort: "high",
    providerProfile: "secondary-provider",
  });
  expect(request?.roles.research_worker).toBe("secondary");
});

test("reselecting the saved API key preserves its reference and loaded models", async ({
  page,
}) => {
  await mountSetup(page, true);
  await page
    .getByRole("button", { name: "Advanced model setup", exact: true })
    .click();
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  await page.getByRole("combobox", { name: "API key", exact: true }).click();
  await page
    .getByRole("option", { name: "Use saved API key", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: /Compact Reasoner/ }),
  ).toBeVisible();
  await expect(
    page.getByLabel("Connection ID", { exact: true }),
  ).toBeEditable();
  await page
    .getByLabel("Connection ID", { exact: true })
    .fill("renamed-provider");
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const request = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: {
            command: string;
            args: { request: { providers: unknown[]; models: unknown[] } };
          }[];
        }
      ).setupRequests.find(
        (entry) => entry.command === "apply_managed_model_configuration",
      )?.args.request,
  );
  expect(request?.providers[0]).toMatchObject({
    profile: "renamed-provider",
    credentialAction: "reuse",
    credentialId: "saved-credential",
  });
  expect(request?.models[0]).toMatchObject({
    providerProfile: "renamed-provider",
    model: "vendor/reasoner",
  });
});

test("adding a key to a saved provider without one enrolls it before reuse", async ({
  page,
}) => {
  await mountSetup(page, true, true, false);
  const noKey = page.getByRole("checkbox", {
    name: "Connect without an API key",
    exact: true,
  });
  await expect(noKey).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: /Use a different API key/ }),
  ).toHaveCount(0);
  await noKey.uncheck();
  await expect(
    page.getByText(/enter your API key in a separate secure window/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await page.getByRole("button", { name: /Compact Reasoner/ }).click();
  await expect(
    page.getByText(/Your saved API key will be reused/),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Refresh models", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: /Compact Reasoner/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByText("Configuration saved", { exact: true }),
  ).toBeVisible();
  const requests = await page.evaluate(
    () =>
      (
        window as unknown as {
          setupRequests: { command: string; args: { request: unknown } }[];
        }
      ).setupRequests,
  );
  const discoveries = requests
    .filter((entry) => entry.command === "discover_managed_provider_models")
    .map((entry) => entry.args.request);
  expect(discoveries[0]).toMatchObject({
    credentialAction: "replace",
    providerProfile: "primary-provider",
  });
  expect(discoveries[0]).not.toHaveProperty("credentialId");
  expect(discoveries[1]).toMatchObject({
    credentialAction: "reuse",
    credentialId: "saved-credential",
  });
  expect(
    requests.find((entry) => entry.command === "configure_managed_runtime")
      ?.args.request,
  ).toMatchObject({
    credentialId: "saved-credential",
    replaceCredential: false,
    noCredential: false,
  });
});

for (const change of ["endpoint", "protocol"] as const) {
  test(`changing a saved provider ${change} requests a new key before loading models`, async ({
    page,
  }) => {
    await mountSetup(page, true);
    await page
      .getByRole("button", { name: "Advanced model setup", exact: true })
      .click();
    await expect(
      page.getByRole("combobox", { name: "API key", exact: true }),
    ).toContainText("Use saved API key");
    if (change === "protocol") {
      await page
        .getByRole("combobox", { name: "API format", exact: true })
        .click();
      await page
        .getByRole("option", { name: "OpenAI Responses", exact: true })
        .click();
      await expect(
        page.getByRole("combobox", { name: "API key", exact: true }),
      ).toContainText("Enter or replace API key");
    }
    await page
      .getByLabel("API base URL", { exact: true })
      .fill(
        change === "endpoint"
          ? "https://new.example.test/v1"
          : "https://openrouter.ai/api/v1",
      );
    await expect(
      page.getByRole("combobox", { name: "API key", exact: true }),
    ).toContainText("Enter or replace API key");
    await page
      .getByRole("button", { name: "Load models", exact: true })
      .click();
    await page.getByRole("button", { name: /Compact Reasoner/ }).click();
    const request = await page.evaluate(
      () =>
        (
          window as unknown as {
            setupRequests: { command: string; args: { request: unknown } }[];
          }
        ).setupRequests.find(
          (entry) => entry.command === "discover_managed_provider_models",
        )?.args.request,
    );
    expect(request).toMatchObject({
      credentialAction: "replace",
      providerKind:
        change === "protocol" ? "openai_responses" : "openai_compatible",
    });
    expect(request).not.toHaveProperty("credentialId");
  });
}

test("a rejected key in fresh setup can be replaced without reusing its saved reference", async ({
  page,
}) => {
  await mountSetup(page);
  await page.evaluate(() => {
    (window as unknown as { failCatalog: boolean }).failCatalog = true;
  });
  await page.getByRole("button", { name: "Load models", exact: true }).click();
  await expect(
    page.locator(".provider-model-picker").getByRole("alert"),
  ).toContainText("Authentication failed");
  for (let attempt = 0; attempt < 2; attempt += 1) {
    await page
      .getByRole("button", { name: "Use a different API key", exact: true })
      .click();
    await expect(
      page.getByText(/enter your API key in a separate secure window/),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Load models", exact: true })
      .click();
    await expect(
      page.locator(".provider-model-picker").getByRole("alert"),
    ).toContainText("Authentication failed");
  }
  const requests = await page.evaluate(() =>
    (
      window as unknown as {
        setupRequests: { command: string; args: { request: unknown } }[];
      }
    ).setupRequests
      .filter((entry) => entry.command === "discover_managed_provider_models")
      .map((entry) => entry.args.request),
  );
  expect(requests).toHaveLength(3);
  for (const request of requests) {
    expect(request).toMatchObject({ credentialAction: "replace" });
    expect(request).not.toHaveProperty("credentialId");
  }
});

for (const advanced of [false, true]) {
  test(`reselecting the current provider and API format preserves ${advanced ? "advanced" : "basic"} setup`, async ({
    page,
  }) => {
    await mountSetup(page);
    await choosePreset(page, "Custom Responses");
    await page
      .getByLabel("API base URL", { exact: true })
      .fill("https://custom.example.test/v1");
    if (advanced)
      await page
        .getByRole("button", { name: "Advanced model setup", exact: true })
        .click();
    await page
      .getByRole("button", { name: "Load models", exact: true })
      .click();
    await page.getByRole("button", { name: /Compact Reasoner/ }).click();
    if (!advanced)
      await page
        .getByText("Model limits and capabilities", { exact: true })
        .click();
    await page
      .getByLabel("Maximum output (tokens)", { exact: true })
      .fill("2048");
    for (const selection of ["provider", "format"]) {
      if (selection === "provider")
        await choosePreset(page, "Custom Responses");
      else {
        await page
          .getByRole("combobox", { name: "API format", exact: true })
          .click();
        await page
          .getByRole("option", { name: "OpenAI Responses", exact: true })
          .click();
      }
      await expect(
        page.getByLabel("API base URL", { exact: true }),
      ).toHaveValue("https://custom.example.test/v1");
      await expect(
        page.getByLabel(advanced ? "Provider model ID" : "Model ID", {
          exact: true,
        }),
      ).toHaveValue("vendor/reasoner");
      await expect(
        page.getByLabel("Maximum output (tokens)", { exact: true }),
      ).toHaveValue("2048");
      await expect(
        page.getByRole("button", { name: /Compact Reasoner/ }),
      ).toBeVisible();
    }
    await page
      .getByRole("button", { name: "Save and start", exact: true })
      .click();
    await expect(
      page.getByText("Configuration saved", { exact: true }),
    ).toBeVisible();
    const request = await page.evaluate(
      (advanced) =>
        (
          window as unknown as {
            setupRequests: {
              command: string;
              args: { request: Record<string, unknown> };
            }[];
          }
        ).setupRequests.find(
          (entry) =>
            entry.command ===
            (advanced
              ? "apply_managed_model_configuration"
              : "configure_managed_runtime"),
        )?.args.request,
      advanced,
    );
    const provider = advanced ? (request?.providers as unknown[])[0] : request;
    expect(provider).toMatchObject({
      credentialId: "saved-credential",
      baseUrl: "https://custom.example.test/v1",
    });
  });
}
