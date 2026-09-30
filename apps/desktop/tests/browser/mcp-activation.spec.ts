import { expect, test, type Page } from "@playwright/test";

async function openMcp(
  page: Page,
  mode: "new" | "updated" | "restarting" | "alternative" = "new",
) {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/?fixture=operations-studio");
  await page.evaluate(async (mode) => {
    const modulePath = "/src/components/ManagedSettingsPane.tsx";
    const { buildManagedSettingsFixture } = await import(modulePath);
    const snapshot = buildManagedSettingsFixture({
      selectedSpaceId: "fixture-managed-local",
      spaces: [
        {
          spaceId: "fixture-managed-local",
          displayName: "Colossus",
          displayPath: "~/Colossus",
          archived: false,
        },
      ],
      managedModelConfiguration: { providers: [], models: [], roles: {} },
      accessProfile: "development",
      executionBoundary: "workspace_isolated",
    });
    const space = snapshot.spaces[0];
    space.pendingGlobalRevision = null;
    space.status = mode === "restarting" ? "restarting" : "active";
    space.configuration.acceptedGlobalRevision =
      snapshot.globalConfiguration.revision;
    const entry = snapshot.globalConfiguration.mcpServers.find(
      (entry: any) => entry.label === "splunk-search",
    );
    entry.revisions[0].value.oauth = {
      clientId: "demo",
      clientSecretCredentialId: null,
      callbackPort: 8765,
      scopes: [],
    };
    if (mode === "alternative") {
      const alternative = structuredClone(entry);
      alternative.id = "alternative-mcp";
      alternative.label = "Other MCP option";
      snapshot.globalConfiguration.mcpServers.unshift(alternative);
    }
    if (mode !== "new") {
      space.configuration.catalogRevisions[`mcp:${entry.id}`] = {
        resourceId: entry.id,
        revision: entry.currentRevision,
      };
    }
    if (mode === "updated") {
      entry.currentRevision += 1;
      entry.revisions.push({
        revision: entry.currentRevision,
        value: {
          ...entry.revisions[0].value,
          url: "https://new.example.test/mcp",
        },
      });
    }
    const host = window as unknown as Record<string, any>;
    host.mcpActivationCalls = [];
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any) => {
        if (command === "list_setup_packages") return [];
        if (command === "get_managed_configuration")
          return structuredClone(snapshot);
        if (command === "sync_managed_configuration") return null;
        host.mcpActivationCalls.push({ command, args });
        if (command === "save_space_configuration") {
          return new Promise((resolve, reject) => {
            host.finishMcpApply = (success: boolean) => {
              if (!success) {
                reject({
                  code: "unavailable",
                  message:
                    "Runtime restart failed. Retry applying the workspace changes.",
                  retryable: true,
                  outcomeUnknown: false,
                  violations: [],
                });
                return;
              }
              for (const key of Object.keys(
                space.configuration.catalogRevisions,
              )) {
                if (key.startsWith("mcp:"))
                  delete space.configuration.catalogRevisions[key];
              }
              for (const resourceId of args.request.selectedMcpResourceIds) {
                const resource = snapshot.globalConfiguration.mcpServers.find(
                  (candidate: any) => candidate.id === resourceId,
                );
                space.configuration.catalogRevisions[`mcp:${resourceId}`] = {
                  resourceId,
                  revision: resource.currentRevision,
                };
              }
              space.status = "active";
              resolve(structuredClone(snapshot));
            };
          });
        }
        if (command === "diagnose_managed_mcp_server") {
          return { server: args.request.server, healthy: true, tools: [] };
        }
        throw new Error(`Unexpected MCP activation command: ${command}`);
      },
    };
  }, mode);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "MCP", exact: true })
    .click();
  return page
    .locator(".managed-mcp-resource")
    .filter({ hasText: "splunk-search" });
}

test("new MCP tests and OAuth wait for Apply and runtime restart, including failure and retry", async ({
  page,
}, testInfo) => {
  const server = await openMcp(page);
  const toggle = server.getByRole("switch");
  const testButton = server.getByRole("button", { name: "Test", exact: true });
  const oauth = server.getByRole("button", { name: "OAuth", exact: true });
  await expect(testButton).toBeDisabled();
  await toggle.check();
  await expect(
    server.getByText("Pending changes", { exact: true }),
  ).toBeVisible();
  await expect(
    server.getByText(
      "Apply workspace changes to restart the runtime before testing.",
    ),
  ).toBeVisible();
  await expect(testButton).toBeDisabled();
  await expect(oauth).toBeDisabled();
  await page.screenshot({
    path: testInfo.outputPath("mcp-pending-activation.png"),
  });
  await expect(
    page
      .locator(".managed-mcp-resource")
      .filter({ hasText: "github-local" })
      .getByRole("button", { name: "Test", exact: true }),
  ).toBeEnabled();
  expect(await page.evaluate(() => (window as any).mcpActivationCalls)).toEqual(
    [],
  );
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(toggle).not.toBeChecked();
  await expect(
    server.getByText("Pending changes", { exact: true }),
  ).toHaveCount(0);
  await toggle.check();
  await page
    .getByRole("button", { name: "Apply Workspace changes", exact: true })
    .click();
  await expect(testButton).toBeDisabled();
  await expect(oauth).toBeDisabled();
  await expect(toggle).toBeDisabled();
  await page.evaluate(() => (window as any).finishMcpApply(false));
  await expect(page.getByRole("alert")).toContainText(
    "Runtime restart failed.",
  );
  await expect(toggle).toBeEnabled();
  await expect(testButton).toBeDisabled();
  await expect(oauth).toBeDisabled();
  await expect(
    server.getByText("Pending changes", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Apply Workspace changes", exact: true })
    .click();
  await page.evaluate(() => (window as any).finishMcpApply(true));
  await expect(testButton).toBeEnabled();
  await expect(oauth).toBeEnabled();
  await expect(
    server.getByText("Pending changes", { exact: true }),
  ).toHaveCount(0);
  await testButton.click();
  await expect(
    server.getByText("Connection healthy", { exact: true }),
  ).toBeVisible();
  const calls = await page.evaluate(() => (window as any).mcpActivationCalls);
  expect(calls.map((call: any) => call.command)).toEqual([
    "save_space_configuration",
    "save_space_configuration",
    "diagnose_managed_mcp_server",
  ]);
  expect(calls.at(-1).args).toEqual({
    request: { spaceId: "fixture-managed-local", server: "splunk-search" },
  });
  await toggle.uncheck();
  await expect(testButton).toBeDisabled();
  await expect(oauth).toBeDisabled();
  await expect(
    server.getByText("Connection healthy", { exact: true }),
  ).toHaveCount(0);
  await expect(
    server.getByText("Pending changes", { exact: true }),
  ).toBeVisible();
});

test("a changed global MCP revision cannot test the older pinned connection", async ({
  page,
}) => {
  const server = await openMcp(page, "updated");
  await expect(server.getByRole("switch")).toBeChecked();
  await expect(
    server.getByRole("button", { name: "Test", exact: true }),
  ).toBeDisabled();
  await expect(
    server.getByRole("button", { name: "OAuth", exact: true }),
  ).toBeDisabled();
  await expect(
    server.getByText("Pending changes", { exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).mcpActivationCalls)).toEqual(
    [],
  );
});

test("a saved MCP connection cannot test while its workspace restarts", async ({
  page,
}) => {
  const server = await openMcp(page, "restarting");
  await expect(server.getByRole("switch")).toBeChecked();
  await expect(
    server.getByRole("button", { name: "Test", exact: true }),
  ).toBeDisabled();
  await expect(
    server.getByRole("button", { name: "OAuth", exact: true }),
  ).toBeDisabled();
  await expect(
    server.getByText("Tests require an active workspace."),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).mcpActivationCalls)).toEqual(
    [],
  );
});

test("tests the selected MCP resource when an earlier catalog alternative shares its server name", async ({
  page,
}) => {
  const server = await openMcp(page, "alternative");
  const alternative = page
    .locator(".managed-mcp-resource")
    .filter({ hasText: "Other MCP option" });
  await expect(alternative.getByRole("switch")).not.toBeChecked();
  await expect(
    alternative.getByRole("button", { name: "Test", exact: true }),
  ).toBeDisabled();
  await expect(server.getByRole("switch")).toBeChecked();
  await server.getByRole("button", { name: "Test", exact: true }).click();
  await expect(
    server.getByText("Connection healthy", { exact: true }),
  ).toBeVisible();
  await expect(
    alternative.getByText("Connection healthy", { exact: true }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).mcpActivationCalls)).toEqual(
    [
      {
        command: "diagnose_managed_mcp_server",
        args: {
          request: {
            spaceId: "fixture-managed-local",
            server: "splunk-search",
          },
        },
      },
    ],
  );
});
