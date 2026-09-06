import { readFileSync } from "node:fs";
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  const icon = `data:image/png;base64,${readFileSync(new URL("../../../../bundled-plugins/colossus/com.obscuritylabs.colossus/icon.png", import.meta.url)).toString("base64")}`;
  await page.addInitScript((icon) => {
    const core = {
      icon_data_url: icon,
      manifest: {
        name: "colossus",
        version: "0.10.10-preview.10",
        description: "Core authoring and development skills",
      },
      digest: `sha256:${"1".repeat(64)}`,
      source: "bundled:colossus",
      origin: "bundled",
      status: "enabled",
      available: true,
      unavailable_reason: null,
      actions: ["inspect", "verify", "export", "enable", "disable"],
      trust: {
        trusted: false,
        profile: null,
        signer: null,
        method: "bundled-executable",
      },
      skills: [
        "coding",
        "offline-dev",
        "security-review",
        "plugin-authoring",
      ].map((name) => ({
        id: `colossus/${name}`,
        plugin: "colossus",
        name,
        description: `Help with ${name}`,
        compatibility: null,
        allowed_tools: null,
      })),
      mcp_servers: [
        {
          id: "colossus/docs",
          name: "docs",
          transport: "streamable-http",
          enabled: false,
          status: "Requires explicit runtime enablement",
        },
      ],
      diagnostics: [],
    };
    const imported = {
      ...core,
      icon_data_url: null,
      manifest: {
        name: "example",
        version: "1.0.0",
        description: "Unsigned candidate",
      },
      origin: "installed",
      digest: `sha256:${"2".repeat(64)}`,
      source: "oci:registry.example/plugin",
      status: "disabled",
      available: false,
      unavailable_reason: "Plugin is not enabled globally",
      skills: [],
      actions: [...core.actions, "update", "uninstall"],
      trust: {
        trusted: false,
        method: "digest-only",
        profile: "offline",
        signer: null,
      },
    };
    const state = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      pluginCalls: { command: string; args: Record<string, unknown> }[];
      pluginFailure?: string;
      pluginPending?: (value: unknown) => void;
      pluginMcpEnabled?: boolean;
    };
    state.pluginCalls = [];
    state.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: Record<string, unknown>) => {
        state.pluginCalls.push({ command, args });
        if (command === "get_plugin_inventory") {
          core.mcp_servers[0]!.enabled = state.pluginMcpEnabled === true;
          return {
            plugins: [core, imported],
            managementAvailable: args.targetId === "local",
          };
        }
        if (command === "managed_mcp_oauth_status")
          return {
            server: "colossus/docs",
            configured: true,
            authenticated: false,
          };
        if (command === "begin_managed_mcp_oauth")
          return {
            server: "colossus/docs",
            authorizationUrl: "https://auth.example.test/authorize",
            callbackUrl: "http://127.0.0.1:8765/callback",
          };
        if (command === "complete_managed_mcp_oauth")
          return {
            server: "colossus/docs",
            configured: true,
            authenticated: true,
          };
        if (command === "logout_managed_mcp_oauth")
          return {
            server: "colossus/docs",
            configured: true,
            authenticated: false,
          };
        if (command === "diagnose_managed_mcp_server")
          return { server: "colossus/docs", healthy: true, tools: [] };
        if (command === "read_plugin_preview") {
          const request = args.request as {
            kind: string;
            skillId: string;
            path?: string;
          };
          if (request.kind === "skill")
            return {
              instructions:
                "Author in the selected workspace, then validate and package.",
              digest: core.digest,
            };
          if (request.kind === "resources")
            return [
              {
                skill_id: request.skillId,
                path: "references/oci.md",
                size: 25,
                text: true,
              },
              {
                skill_id: request.skillId,
                path: "assets/image.bin",
                size: 8,
                text: false,
              },
            ];
          return {
            path: request.path,
            content: "One whole plugin per OCI artifact.",
          };
        }
        if (command === "cancel_plugin_operation") {
          state.pluginPending?.({ cancelled: true });
          return null;
        }
        if (command === "manage_plugin") {
          if (state.pluginFailure === "wait")
            return new Promise((resolve) => {
              state.pluginPending = resolve;
            });
          if (state.pluginFailure)
            throw {
              code: "permission_denied",
              message: state.pluginFailure,
              retryable: false,
              outcomeUnknown: false,
              violations: [],
            };
          const request = (
            args.input as {
              request: { operation: string; name?: string; digest?: string };
            }
          ).request;
          const plugin = request.name === "colossus" ? core : imported;
          if (request.operation === "disable") {
            plugin.status = "disabled";
            plugin.available = false;
          }
          if (request.operation === "enable") {
            plugin.status = "enabled";
            plugin.available = true;
          }
          return {
            integrity:
              request.operation === "verify_installed" ? "verified" : undefined,
          };
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  }, icon);
  await page.goto("/?fixture=plugin-studio");
  await expect(
    page.getByRole("button", { name: /colossus 0\.10/u }),
  ).toBeVisible();
});

test("plugin MCP diagnostics and OAuth require explicit server enablement", async ({
  page,
}) => {
  await page.getByRole("button", { name: /colossus 0\.10/u }).click();
  const controls = page.getByRole("group", {
    name: "colossus/docs connection",
  });
  await expect(
    controls.getByRole("button", { name: "OAuth status" }),
  ).toBeDisabled();
  await expect(
    controls.getByRole("button", { name: "Test connection" }),
  ).toBeDisabled();
  await page.evaluate(() => {
    (window as unknown as { pluginMcpEnabled: boolean }).pluginMcpEnabled =
      true;
  });
  await page
    .getByRole("button", { name: "Refresh plugins", exact: true })
    .click();
  await controls.getByRole("button", { name: "Test connection" }).click();
  await expect(
    controls.getByText("0 allowlisted tools discovered."),
  ).toBeVisible();
  await controls.getByRole("button", { name: "OAuth status" }).click();
  await expect(controls.getByText("Signed out", { exact: true })).toBeVisible();
  await controls.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    controls.getByRole("link", { name: "Open authorization" }),
  ).toHaveAttribute("href", "https://auth.example.test/authorize");
  await controls
    .getByLabel("OAuth callback URL")
    .fill("http://127.0.0.1:8765/callback?code=fixture");
  await controls.getByRole("button", { name: "Complete sign-in" }).click();
  await expect(controls.getByText("Signed in", { exact: true })).toBeVisible();
  await controls.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(controls.getByText("Signed out", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { pluginCalls: { command: string }[] }
      ).pluginCalls.filter(({ command }) => command === "manage_plugin"),
    ),
  ).toEqual([]);
});

test("metadata-only discovery, bounded previews, binary paths, selection, and compact accessibility", async ({
  page,
}) => {
  await page.getByRole("button", { name: /colossus 0\.10/u }).click();
  await expect(
    page.getByRole("heading", {
      name: "colossus/plugin-authoring",
      exact: true,
    }),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { pluginCalls: { command: string }[] }
      ).pluginCalls.filter((call) => call.command === "read_plugin_preview"),
    ),
  ).toEqual([]);
  const skill = page.locator(".plugin-skill").filter({
    has: page.getByRole("heading", {
      name: "colossus/plugin-authoring",
      exact: true,
    }),
  });
  await skill.getByRole("button", { name: "Preview instructions" }).click();
  await expect(
    skill.getByText(
      "Author in the selected workspace, then validate and package.",
    ),
  ).toBeVisible();
  await skill.getByRole("button", { name: "Browse resources" }).click();
  await expect(
    skill.getByText("assets/image.bin", { exact: true }),
  ).toBeVisible();
  await expect(
    skill.getByRole("button", { name: "assets/image.bin" }),
  ).toHaveCount(0);
  await skill.getByRole("button", { name: "references/oci.md" }).click();
  await expect(
    skill.getByText("One whole plugin per OCI artifact."),
  ).toBeVisible();
  await skill.getByRole("button", { name: "Use in this conversation" }).click();
  await expect(
    skill.getByRole("button", { name: "Selected for this conversation" }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "New conversation", exact: true })
    .click();
  await expect(
    skill.getByRole("button", { name: "Use in this conversation" }),
  ).toBeEnabled();
  await page.setViewportSize({ width: 520, height: 800 });
  await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 520);
  const results = await new AxeBuilder({ page })
    .include(".plugin-surface")
    .analyze();
  expect(
    results.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.screenshot({
    path: "../../output/playwright/plugins-compact.png",
    fullPage: true,
  });
});

test("core ownership, global lifecycle, untrusted activation, explicit digest, and errors", async ({
  page,
}) => {
  await page.getByRole("button", { name: /colossus 0\.10/u }).click();
  const detail = page.getByRole("article", { name: "colossus details" });
  await expect(
    detail.getByRole("button", { name: "Uninstall", exact: true }),
  ).toHaveCount(0);
  await expect(
    detail.getByRole("button", { name: "Update", exact: true }),
  ).toHaveCount(0);
  await detail.getByRole("button", { name: "Disable", exact: true }).click();
  await page.getByRole("button", { name: "Continue disable" }).click();
  await expect(
    detail.getByRole("button", { name: "Activate this digest" }),
  ).toBeVisible();
  await page.getByRole("button", { name: /example 1\.0/u }).click();
  await page.getByRole("button", { name: "Activate this digest" }).click();
  await page.getByRole("checkbox", { name: /Request approval/u }).check();
  await page.evaluate(() => {
    (window as unknown as { pluginFailure: string }).pluginFailure =
      "Policy denied plugin.enable";
  });
  await page.getByRole("button", { name: "Continue enable" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Policy denied plugin.enable",
  );
  const calls = await page.evaluate(
    () =>
      (
        window as unknown as {
          pluginCalls: { command: string; args: unknown }[];
        }
      ).pluginCalls,
  );
  expect(
    calls.filter((call) => call.command === "manage_plugin").at(-1)?.args,
  ).toMatchObject({
    targetId: "local",
    input: {
      request: {
        operation: "enable",
        name: "example",
        digest: `sha256:${"2".repeat(64)}`,
        allow_untrusted: true,
      },
    },
  });
});

for (const action of [
  "install",
  "validate",
  "verify",
  "package",
  "pull",
  "push",
  "gc",
] as const) {
  test(`management ${action} translates a typed native request`, async ({
    page,
  }) => {
    if (action !== "install")
      await page.getByText("Developer tools", { exact: true }).click();
    const actions = action === "install" ? page : page.locator(".plugin-tools");
    await actions
      .getByRole("button", {
        name:
          action === "gc"
            ? "Garbage collect"
            : action[0]!.toUpperCase() + action.slice(1),
        exact: true,
      })
      .click();
    const form = page.getByRole("form", { name: `${action} plugin` });
    await expect(form).toBeFocused();
    if (action === "pull" || action === "push") {
      await form
        .getByLabel("Registry profile", { exact: true })
        .fill("internal");
      await form
        .getByLabel("Registry reference")
        .fill("registry.example/team/plugin:v1");
    }
    await form.getByRole("button", { name: `Continue ${action}` }).click();
    await expect(
      page.getByRole("status").filter({ hasText: `${action} completed` }),
    ).toBeVisible();
    const requests = await page.evaluate(() =>
      (
        window as unknown as {
          pluginCalls: { command: string; args: unknown }[];
        }
      ).pluginCalls.filter((call) => call.command === "manage_plugin"),
    );
    expect(requests.at(-1)?.args).toMatchObject({
      targetId: "local",
      input: { request: { operation: action } },
    });
  });
}

test("archive candidates, cancellation, external discovery and unavailable capability", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Install", exact: true }).click();
  await page.getByRole("combobox", { name: "Installation source" }).click();
  await page
    .getByRole("option", { name: "OCI layout archive", exact: true })
    .click();
  await page
    .getByLabel("Exact manifest digest")
    .fill(`sha256:${"3".repeat(64)}`);
  await page.evaluate(() => {
    (window as unknown as { pluginFailure: string }).pluginFailure = "wait";
  });
  await page.getByRole("button", { name: "Continue install" }).click();
  await page.getByRole("button", { name: "Cancel operation" }).click();
  await expect(
    page.getByRole("button", { name: "Install", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "External", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Install", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText(
      "Read-only discovery. Lifecycle management is available on Managed Local.",
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: "Unsupported target" }).click();
  await expect(
    page.getByText(
      /This target does not advertise authorized plugin discovery/u,
    ),
  ).toBeVisible();
});

test("typed settings preserve disabled MCP, explicit credentials, trust, and Docker selection", async ({
  page,
}) => {
  await page.getByLabel("New plugins.mcpServers key").fill("example/server");
  await page
    .getByRole("button", { name: "Add plugins.mcpServers entry" })
    .click();
  await expect(
    page.getByLabel("Explicitly enable this MCP server"),
  ).not.toBeChecked();
  await page
    .getByLabel("Allowed tool names (or a sole *)")
    .fill("search\nread");
  await page
    .getByLabel("New Environment credential references key")
    .fill("API_KEY");
  await page
    .getByRole("button", {
      name: "Add Environment credential references entry",
    })
    .click();
  await page.getByLabel("API_KEY", { exact: true }).fill("host:plugin-key");
  await expect(
    page.getByLabel("Explicitly enable this MCP server"),
  ).not.toBeChecked();
  await page.getByLabel("New plugins.trustProfiles key").fill("offline");
  await page
    .getByRole("button", { name: "Add plugins.trustProfiles entry" })
    .click();
  await page.getByRole("combobox", { name: "Signature policy" }).click();
  await page.getByRole("option", { name: "optional", exact: true }).click();
  await page.getByLabel("New plugins.registries key").fill("internal");
  await page
    .getByRole("button", { name: "Add plugins.registries entry" })
    .click();
  await page.getByRole("combobox", { name: "Credential source" }).click();
  await page.getByRole("option", { name: "docker", exact: true }).click();
  await page
    .getByLabel("Exact registry origin")
    .fill("https://registry.example");
  await page
    .getByLabel("Docker config path")
    .fill("/opt/credentials/docker.json");
  const settings = JSON.parse(
    await page.getByLabel("Test settings value").innerText(),
  );
  expect(settings["plugins.mcpServers"]["example/server"]).toMatchObject({
    enabled: false,
    allowedTools: ["search", "read"],
    environment: { API_KEY: "host:plugin-key" },
  });
  expect(settings["plugins.registries"].internal.auth).toEqual({
    kind: "docker",
    configPath: "/opt/credentials/docker.json",
  });
  expect(settings["plugins.trustProfiles"].offline.mode).toBe("optional");
});

test("plugin icons, availability filters, search recovery, and installation disclosure", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  const detail = page.getByRole("article", { name: "colossus details" });
  const icon = detail.locator(".plugin-icon img");
  await expect(icon).toBeVisible();
  await expect(icon).toHaveJSProperty("naturalWidth", 128);
  await expect(page.locator(".plugin-digest")).toBeHidden();
  await page.getByText("Installation details", { exact: true }).click();
  await expect(page.locator(".plugin-digest")).toBeVisible();
  await page.getByText("Installation details", { exact: true }).click();
  await page.locator(".plugin-surface").screenshot({
    path: "../../output/playwright/plugins-desktop.png",
    animations: "disabled",
  });
  await page
    .getByRole("button", { name: "Unavailable 1", exact: true })
    .click();
  await expect(page.locator(".plugin-card")).toHaveCount(1);
  await expect(
    page.getByRole("article", { name: "example details" }),
  ).toBeVisible();
  await expect(page.locator(".plugin-card .plugin-icon")).toHaveText("EX");
  await page.getByLabel("Search plugins and skills").fill("no-such-plugin");
  await expect(
    page.getByText("No matching plugins", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(page.locator(".plugin-card")).toHaveCount(2);
  await page.getByLabel("Search plugins and skills").fill("authoring");
  await expect(page.locator(".plugin-card")).toHaveCount(1);
  await expect(detail).toBeVisible();
  await page.evaluate(() => (document.documentElement.dataset.theme = "dark"));
  await page.locator(".plugin-surface").screenshot({
    path: "../../output/playwright/plugins-dark.png",
    animations: "disabled",
  });
  const results = await new AxeBuilder({ page })
    .include(".plugin-surface")
    .analyze();
  expect(
    results.violations.filter(({ impact }) =>
      ["critical", "serious"].includes(impact ?? ""),
    ),
  ).toEqual([]);
});
