import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import sample from "../../src/dev/setup-package-preview.json" with { type: "json" };

async function setup(
  page: Page,
  options: { preloaded?: boolean; busyCount?: number } = {},
) {
  await page.addInitScript(
    ({ sample, options }) => {
      const host = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        setupCalls: { command: string; args: Record<string, unknown> }[];
      };
      host.setupCalls = [];
      let imported: (typeof packet)[] = options.preloaded
        ? [structuredClone(sample)]
        : [];
      let busyCount = options.busyCount ?? 0;
      const packet = {
        ...sample,
        certificateFingerprints: ["12".repeat(32)],
        existingCertificateFingerprints: ["34".repeat(32)],
      };
      packet.providers[0]!.descriptionMarkdown +=
        "\n\n![Remote](https://untrusted.example.com/tracking.png)\n<script>window.injected=true</script>";
      host.__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          host.setupCalls.push({ command, args });
          if (command === "list_setup_packages") {
            if (busyCount-- > 0)
              throw {
                code: "busy",
                message: "Initialization is in progress.",
                retryable: true,
                outcomeUnknown: false,
                violations: [],
              };
            return structuredClone(imported);
          }
          if (command === "inspect_setup_package")
            return { ...packet, replacesVersion: imported.length ? "2" : null };
          if (command === "apply_setup_package") {
            imported = [structuredClone(packet)];
            return null;
          }
          if (command === "desktop_status") return {};
          if (command === "get_managed_configuration")
            return { globalConfiguration: { credentials: [] } };
          if (command === "configure_setup_credential") {
            const profile = (args.request as { profile: string }).profile;
            const provider = imported[0]!.providers.find(
              (entry) => entry.profile === profile,
            )!;
            (provider as { credentialId: string | null }).credentialId =
              "saved-native-handle";
            return null;
          }
          if (command === "get_provider_presets")
            return [
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
            ];
          if (
            command === "apply_managed_model_configuration" ||
            command === "open_setup_link" ||
            command === "cancel_setup_package_review"
          )
            return null;
          throw new Error("Unexpected setup command: " + command);
        },
      };
    },
    { sample, options },
  );
  await page.goto("/?fixture=setup");
  await expect(
    page.getByRole("button", { name: "Import setup file", exact: true }),
  ).toBeVisible();
}
const importButton = (page: Page) =>
  page.getByRole("button", {
    name: "Import setup",
    exact: true,
  });
const next = (page: Page) =>
  page.getByRole("button", { name: "Continue", exact: true });
async function importPackage(page: Page) {
  await page
    .getByRole("button", { name: "Import setup file", exact: true })
    .click();
  await importButton(page).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
}
async function toProviders(page: Page) {
  await next(page).click();
  await expect(
    page.getByRole("heading", { name: "Choose your first workspace" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Choose folder", exact: true })
    .click();
  await expect(
    page.getByRole("radio", { name: "Company AI", exact: true }),
  ).toBeChecked();
}
async function calls(page: Page) {
  return page.evaluate(
    () =>
      (
        window as unknown as {
          setupCalls: { command: string; args: Record<string, unknown> }[];
        }
      ).setupCalls,
  );
}

test("import is offline, review is accessible, and Desktop stays focused on desktop settings", async ({
  page,
}) => {
  const remote: string[] = [];
  page.on("request", (request) => {
    if (
      !request.url().startsWith("http://127.0.0.1") &&
      !request.url().startsWith("data:")
    )
      remote.push(request.url());
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await setup(page);
  await page
    .getByRole("button", { name: "Import setup file", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Review Company AI setup" });
  await expect(dialog).toBeVisible();
  const defaults = dialog.getByRole("checkbox", {
    name: "Use included global defaults",
  });
  await expect(defaults).not.toBeChecked();
  await expect(
    dialog.getByRole("region", { name: "MCP servers", exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("region", { name: "Search providers", exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("region", { name: "Telemetry profiles", exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByText("Version 2 · 5 providers · 6 models"),
  ).toBeVisible();
  await expect(
    page.getByRole("checkbox", {
      name: "Trust the included CA certificates in Colossus",
    }),
  ).not.toBeChecked();
  await dialog
    .getByText("View instructions and configuration", { exact: false })
    .first()
    .click();
  await expect(
    dialog.getByText("32,768 / 4,096", { exact: true }).first(),
  ).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Get an API token" }),
  ).toBeVisible();
  await dialog
    .getByRole("button", { name: "AI portal (open in browser)" })
    .click();
  await expect(dialog.locator("script")).toHaveCount(0);
  expect(remote).toEqual([]);
  expect(
    (
      await new AxeBuilder({ page })
        .include("dialog")
        .withTags(["wcag2a", "wcag2aa"])
        .analyze()
    ).violations,
  ).toEqual([]);
  await defaults.check();
  await page.screenshot({
    path: test.info().outputPath("setup-global-review.png"),
  });
  await importButton(page).click();
  await expect(
    page.getByText(
      /File includes 5 providers · 6 models · global defaults.*telemetry profiles/,
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Choose workspace", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Add API key", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByRole("table")).toHaveCount(0);
  await expect(
    page.getByRole("radio", { name: "Light", exact: true }),
  ).toBeVisible();
  const history = await calls(page);
  expect(
    history.find((c) => c.command === "apply_setup_package")?.args.request,
  ).toEqual({
    sha256: sample.sha256,
    trustCertificates: false,
    replaceExisting: false,
    applyDefaults: true,
  });
  expect(
    history.some((c) =>
      /configure_setup_credential|discover_managed_provider_models|apply_managed_model_configuration/.test(
        c.command,
      ),
    ),
  ).toBe(false);
});

test("five providers and bundled models follow the wizard, defer keys, and preserve imported metadata at Start", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await setup(page);
  await importPackage(page);
  await toProviders(page);
  await expect(
    page
      .getByRole("region", { name: "Imported providers", exact: true })
      .getByRole("radio"),
  ).toHaveCount(5);
  await expect(
    page.getByRole("heading", { name: "Get an API token" }),
  ).toBeVisible();
  await expect(
    page.getByLabel("API base URL", { exact: true }),
  ).not.toBeVisible();
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "company/engineering", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("button", { name: "Load models", exact: true }),
  ).not.toBeVisible();
  await page
    .getByRole("radio", { name: "company/general", exact: true })
    .check();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    page.getByRole("radio", { name: "Company AI", exact: true }),
  ).toBeChecked();
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "company/general", exact: true }),
  ).toBeChecked();
  await page
    .getByRole("radio", { name: "company/engineering", exact: true })
    .check();
  await next(page).click();
  expect(
    (await calls(page)).some((c) =>
      /configure_setup_credential|discover_managed_provider_models|apply_managed_model_configuration/.test(
        c.command,
      ),
    ),
  ).toBe(false);
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Setup complete" }),
  ).toBeVisible();
  const request = (await calls(page)).find(
    (c) => c.command === "apply_managed_model_configuration",
  )!.args.request as Record<string, unknown>;
  expect(request.providers).toEqual([
    {
      profile: "company",
      providerKind: "openai_compatible",
      baseUrl: "https://ai.example.com/v1",
      timeoutMs: 30000,
      credentialAction: "reuse",
      credentialId: "saved-native-handle",
    },
  ]);
  expect(request.roles).toEqual({ primary: "engineering" });
  expect(request.models).toEqual(
    expect.arrayContaining(sample.providers[0]!.models),
  );
});

test("switching provider selects its own models and API key actions update the selected row", async ({
  page,
}) => {
  await setup(page);
  await importPackage(page);
  await toProviders(page);
  await page
    .getByRole("radio", { name: "Vision Gateway", exact: true })
    .check();
  await expect(
    page.getByRole("heading", { name: "Connect your account" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Add API key", exact: true }).click();
  await expect(
    page.getByText("Key saved · not checked", { exact: true }),
  ).toBeVisible();
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "vision/reader", exact: true }),
  ).toBeChecked();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await page.getByRole("radio", { name: "Local Models", exact: true }).check();
  await expect(
    page.getByText("No key required", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add API key", exact: true }),
  ).toHaveCount(0);
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "example-local", exact: true }),
  ).toBeChecked();
});

test("review traps focus, restores it on Escape, and requires explicit replacement", async ({
  page,
}) => {
  await setup(page);
  await importPackage(page);
  const trigger = page.getByRole("button", {
    name: "Import setup file",
    exact: true,
  });
  await trigger.click();
  await expect(importButton(page)).toBeDisabled();
  await page.getByRole("checkbox", { name: /Replace saved setup/ }).check();
  await expect(importButton(page)).toBeEnabled();
  await page.getByRole("dialog").press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(trigger).toBeFocused();
  expect(
    (await calls(page)).find(
      (entry) => entry.command === "cancel_setup_package_review",
    )?.args,
  ).toEqual({ sha256: sample.sha256 });
  expect(
    (await calls(page)).filter((c) => c.command === "apply_setup_package"),
  ).toHaveLength(1);
});

for (const theme of ["Light", "Dark"]) {
  test(`imported flow fits a narrow screen in ${theme}`, async ({ page }) => {
    await setup(page);
    await page.setViewportSize({ width: 430, height: 900 });
    await page.getByRole("radio", { name: theme, exact: true }).check();
    await page
      .getByRole("button", { name: "Import setup file", exact: true })
      .click();
    expect(
      (
        await new AxeBuilder({ page })
          .include("dialog")
          .withTags(["wcag2a", "wcag2aa"])
          .analyze()
      ).violations,
    ).toEqual([]);
    await importButton(page).click();
    await toProviders(page);
    expect(
      (
        await new AxeBuilder({ page })
          .include(".onboarding-surface")
          .withTags(["wcag2a", "wcag2aa"])
          .analyze()
      ).violations,
    ).toEqual([]);
    expect(
      await page
        .locator(".onboarding-surface")
        .evaluate((node) => node.scrollWidth <= node.clientWidth + 1),
    ).toBe(true);
    await next(page).click();
    expect(
      (
        await new AxeBuilder({ page })
          .include(".onboarding-surface")
          .withTags(["wcag2a", "wcag2aa"])
          .analyze()
      ).violations,
    ).toEqual([]);
  });
}

test("inspection errors show actionable validation details and allow retry", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const original = host.__TAURI_INTERNALS__.invoke;
    let fail = true;
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "inspect_setup_package" && fail) {
        fail = false;
        throw {
          code: "invalid_argument",
          message: "The request is invalid.",
          retryable: false,
          outcomeUnknown: false,
          violations: [
            {
              field: "setupPackage",
              description: "A referenced icon is missing.",
            },
          ],
        };
      }
      return original(command, args);
    };
  });
  await page
    .getByRole("button", { name: "Import setup file", exact: true })
    .click();
  await expect(page.getByRole("alert")).toHaveText(
    "A referenced icon is missing.",
  );
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Import setup file", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Review Company AI setup" }),
  ).toBeVisible();
});

test("a cancelled deferred key prompt keeps Start retryable and never activates without a key", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const original = host.__TAURI_INTERNALS__.invoke;
    let cancelled = false;
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "configure_setup_credential" && !cancelled) {
        cancelled = true;
        throw {
          code: "cancelled",
          message: "API key entry was cancelled.",
          retryable: false,
          outcomeUnknown: false,
          violations: [],
        };
      }
      return original(command, args);
    };
  });
  await importPackage(page);
  await toProviders(page);
  await next(page).click();
  await next(page).click();
  const start = page.getByRole("button", {
    name: "Save and start",
    exact: true,
  });
  await start.click();
  await expect(
    page.getByRole("alert").filter({ hasText: "API key entry was cancelled." }),
  ).toBeVisible();
  expect(
    (await calls(page)).some(
      (entry) => entry.command === "apply_managed_model_configuration",
    ),
  ).toBe(false);
  await expect(start).toBeEnabled();
  await start.click();
  await expect(
    page.getByRole("heading", { name: "Setup complete" }),
  ).toBeVisible();
  expect(
    (await calls(page)).filter(
      (entry) => entry.command === "configure_setup_credential",
    ),
  ).toHaveLength(1);
});

test("a saved imported key is reused at Start without prompting again", async ({
  page,
}) => {
  await setup(page);
  await importPackage(page);
  await toProviders(page);
  await page.getByRole("button", { name: "Add API key", exact: true }).click();
  await expect(
    page.getByText("Key saved · not checked", { exact: true }),
  ).toBeVisible();
  await next(page).click();
  await next(page).click();
  await page
    .getByRole("button", { name: "Save and start", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Setup complete" }),
  ).toBeVisible();
  const history = await calls(page);
  expect(
    history.filter((entry) => entry.command === "configure_setup_credential"),
  ).toHaveLength(1);
  expect(
    history.find(
      (entry) => entry.command === "apply_managed_model_configuration",
    )?.args.request,
  ).toMatchObject({
    providers: [
      { credentialAction: "reuse", credentialId: "saved-native-handle" },
    ],
  });
});

test("replacing an imported package refreshes the selected connection and model before activation", async ({
  page,
}) => {
  await setup(page);
  await importPackage(page);
  await toProviders(page);
  await page.getByRole("button", { name: "Add API key", exact: true }).click();
  await expect(
    page.getByText("Key saved · not checked", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "1 Desktop", exact: true }).click();
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const original = host.__TAURI_INTERNALS__.invoke;
    let replaced = false;
    const updated = (packet: typeof sample) => ({
      ...packet,
      sha256: "b".repeat(64),
      version: "3",
      providers: packet.providers.map((provider) =>
        provider.profile === "company"
          ? {
              ...provider,
              baseUrl: "https://updated.example.com/v1",
              credentialId: null,
              models: provider.models.map((model) => ({
                ...model,
                model: model.model + "-v3",
              })),
            }
          : provider,
      ),
    });
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const result = await original(command, args);
      if (command === "inspect_setup_package")
        return updated(result as typeof sample);
      if (command === "apply_setup_package") replaced = true;
      if (command === "list_setup_packages" && replaced)
        return (result as (typeof sample)[]).map(updated);
      return result;
    };
  });
  await page
    .getByRole("button", { name: "Import setup file", exact: true })
    .click();
  await page.getByRole("checkbox", { name: /Replace saved setup/ }).check();
  await importButton(page).click();
  await next(page).click();
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "Company AI", exact: true }),
  ).toBeChecked();
  await expect(
    page
      .getByRole("article", { name: "Company AI details" })
      .getByText("https://updated.example.com/v1", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Needs API key", { exact: true })).toBeVisible();
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "company/engineering-v3", exact: true }),
  ).toBeChecked();
  expect(
    (await calls(page)).some(
      (entry) => entry.command === "apply_managed_model_configuration",
    ),
  ).toBe(false);
});

test("saved imports recover from startup contention without choosing builtin defaults", async ({
  page,
}) => {
  await setup(page, { preloaded: true, busyCount: 4 });
  await expect(
    page.getByText(
      /File includes 5 providers · 6 models · global defaults.*telemetry profiles/,
    ),
  ).toBeVisible();
  await toProviders(page);
  await expect(
    page.getByRole("radio", { name: "Company AI", exact: true }),
  ).toBeChecked();
  await next(page).click();
  await expect(
    page.getByRole("radio", { name: "company/engineering", exact: true }),
  ).toBeChecked();
  expect(
    (await calls(page)).filter(
      (entry) => entry.command === "list_setup_packages",
    ).length,
  ).toBeGreaterThan(4);
});
