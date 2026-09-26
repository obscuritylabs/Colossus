import { expect, test, type Page } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
});

async function installNativeSettingsMock(
  page: Page,
  mode: "idle" | "waiting" | "failed",
) {
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
      accessProfile: "allow_all",
      executionBoundary: "full_access",
    });
    const workspace = snapshot.spaces[0];
    workspace.pendingGlobalRevision = null;
    workspace.status = "active";
    workspace.configuration.acceptedGlobalRevision =
      snapshot.globalConfiguration.revision;
    const mock = {
      snapshot,
      mode,
      syncCalls: 0,
      applyCalls: 0,
      failApply: false,
    };
    const host = window as unknown as Record<string, any>;
    host.settingsUpdateMock = mock;
    const apply = () => {
      workspace.pendingGlobalRevision = null;
      workspace.status = "active";
      workspace.statusMessage = "Settings applied";
      workspace.configuration.acceptedGlobalRevision =
        snapshot.globalConfiguration.revision;
      for (const field of snapshot.globalConfiguration.defaults.revisions.at(-1)
        .value.fieldOverrides) {
        const effective = workspace.effectiveValues.find(
          (value: any) => value.fieldId === field.fieldId,
        );
        if (effective) {
          effective.value = field.value;
          effective.source = "global";
        }
      }
    };
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any) => {
        if (command === "get_managed_configuration")
          return structuredClone(snapshot);
        if (command === "save_global_defaults") {
          snapshot.globalConfiguration.revision++;
          const revision = snapshot.globalConfiguration.revision;
          snapshot.globalConfiguration.defaults.currentRevision = revision;
          snapshot.globalConfiguration.defaults.revisions.push({
            revision,
            value: args.request,
          });
          workspace.pendingGlobalRevision = revision;
          workspace.status = "update_available";
          return structuredClone(snapshot);
        }
        if (command === "sync_managed_configuration") {
          mock.syncCalls++;
          if (!workspace.pendingGlobalRevision) return null;
          if (mock.mode === "idle") apply();
          else {
            workspace.status =
              mock.mode === "waiting" ? "update_waiting" : "update_failed";
            workspace.statusMessage =
              mock.mode === "waiting"
                ? "Global changes saved. They will apply automatically when active work finishes."
                : "Global changes are saved but could not be applied. Retry to see the error.";
          }
          return structuredClone(snapshot);
        }
        if (command === "apply_space_configuration") {
          mock.applyCalls++;
          if (mock.failApply)
            throw {
              code: "configuration",
              message: "The provider configuration is unavailable.",
              retryable: true,
              outcomeUnknown: false,
              violations: [],
            };
          apply();
          return structuredClone(snapshot);
        }
        throw new Error(`Unexpected settings test command: ${command}`);
      },
    };
  }, mode);
}

async function saveGlobalTurns(page: Page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Defaults", exact: true })
    .click();
  await page.getByRole("spinbutton", { name: "Maximum turns" }).fill("75");
  await page
    .getByRole("button", { name: "Save global changes", exact: true })
    .click();
}

test("pending global updates and workspace edits share a single footer action", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const footer = page.locator(".managed-settings-actions");
  await expect(
    page
      .locator(".settings-context-actions")
      .getByRole("button", { name: /apply/i }),
  ).toHaveCount(0);
  await expect(
    footer.getByRole("button", { name: "Apply global updates" }),
  ).toBeEnabled();
  await expect(
    footer.getByRole("button", { name: "Apply Workspace changes" }),
  ).toHaveCount(0);
  await page.getByRole("spinbutton", { name: "Maximum turns" }).fill("60");
  await expect(footer).toContainText("Saved global updates will be included");
  await expect(
    footer.getByRole("button", { name: "Apply global updates" }),
  ).toHaveCount(0);
  await footer.getByRole("button", { name: "Apply Workspace changes" }).click();
  await expect(footer).toContainText("All changes saved and applied");
  await expect(footer.getByRole("button")).toHaveCount(0);
});

test("saved global edits apply automatically to idle workspaces", async ({
  page,
}) => {
  await installNativeSettingsMock(page, "idle");
  await saveGlobalTurns(page);
  const footer = page.locator(".managed-settings-actions");
  await expect(footer).toContainText("All changes saved and applied");
  await expect(footer.getByRole("button")).toHaveCount(0);
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await expect(
    page.getByRole("spinbutton", { name: "Maximum turns" }),
  ).toHaveValue("75");
  expect(
    await page.evaluate(() => (window as any).settingsUpdateMock.applyCalls),
  ).toBe(0);
});

test("waiting updates stay visible and a background apply preserves unsaved edits", async ({
  page,
}) => {
  await installNativeSettingsMock(page, "waiting");
  await saveGlobalTurns(page);
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  const footer = page.locator(".managed-settings-actions");
  await expect(footer).toContainText("automatically when active work finishes");
  await expect(footer.getByRole("button")).toHaveCount(0);
  await page.setViewportSize({ width: 700, height: 900 });
  await expect(footer.getByRole("status")).toBeVisible();
  await page.getByRole("spinbutton", { name: "Maximum turns" }).fill("60");
  await page.evaluate(async () => {
    (window as any).settingsUpdateMock.mode = "idle";
    // Reuse Vite's loaded URL, including its HMR timestamp, to notify the UI's listeners.
    const modulePath = performance
      .getEntriesByType("resource")
      .find(
        (entry) =>
          new URL(entry.name).pathname === "/src/managed-settings-updates.ts",
      )?.name;
    if (!modulePath) throw new Error("Settings update module was not loaded");
    await (await import(modulePath)).syncSavedSettings();
  });
  await expect(
    page.getByRole("spinbutton", { name: "Maximum turns" }),
  ).toHaveValue("60");
  await expect(footer).toContainText("Unsaved changes");
  await footer.getByRole("button", { name: "Discard" }).click();
  await expect(
    page.getByRole("spinbutton", { name: "Maximum turns" }),
  ).toHaveValue("75");
  await expect(footer.getByRole("button")).toHaveCount(0);
  await page.screenshot({
    path: "output/playwright/settings-updates-saved.png",
  });
});

test("a failed automatic update offers retry and keeps the native error visible", async ({
  page,
}) => {
  await installNativeSettingsMock(page, "failed");
  await saveGlobalTurns(page);
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  const footer = page.locator(".managed-settings-actions");
  await page.evaluate(() => {
    (window as any).settingsUpdateMock.failApply = true;
  });
  await footer.getByRole("button", { name: "Retry global update" }).click();
  await expect(footer.getByRole("alert")).toContainText(
    "provider configuration is unavailable",
  );
  await page.evaluate(() => {
    (window as any).settingsUpdateMock.failApply = false;
  });
  await footer.getByRole("button", { name: "Retry global update" }).click();
  await expect(footer).toContainText("All changes saved and applied");
  await expect(footer.getByRole("button")).toHaveCount(0);
});
