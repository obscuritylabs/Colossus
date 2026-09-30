import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import sample from "../../src/dev/setup-package-preview.json" with { type: "json" };

async function openInventory(page: Page, imported = true) {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/?fixture=operations-studio");
  await page.evaluate(
    async ({ sample, imported }) => {
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
      const packet = {
        ...sample,
        providers: sample.providers.map((p, index) => ({
          ...p,
          catalogResourceId: `provider-${index}`,
        })),
      };
      let packages = imported ? [packet] : [];
      const populate = () => {
        snapshot.globalConfiguration.providers = packet.providers.map((p) => ({
          id: p.catalogResourceId,
          label: p.displayName,
          currentRevision: 1,
          archived: false,
          revisions: [
            {
              revision: 1,
              value: {
                profile: p.profile,
                kind: p.kind,
                baseUrl: p.baseUrl,
                credentialId: p.credentialId,
                credentialRequired: p.credentialRequired,
                timeoutMs: p.timeoutMs,
              },
            },
          ],
        }));
        snapshot.globalConfiguration.models = packet.providers.flatMap((p) =>
          p.models.map((model) => ({
            id: `model-${model.profile}`,
            label: model.model,
            currentRevision: 1,
            archived: false,
            revisions: [{ revision: 1, value: model }],
          })),
        );
      };
      if (imported) populate();
      const host = window as unknown as Record<string, any>;
      host.setupInventory = {
        calls: [],
        failActivation: true,
        complete: null,
        snapshot,
        refreshes: 0,
      };
      // Keep the surrounding deterministic App fixture; native status refresh is a separate integration boundary.
      window.addEventListener(
        "colossus-setup-refresh",
        (event) => {
          event.stopImmediatePropagation();
          host.setupInventory.refreshes++;
        },
        true,
      );
      host.__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: any = {}) => {
          host.setupInventory.calls.push({ command, args });
          if (command === "get_managed_configuration")
            return structuredClone(snapshot);
          if (command === "list_setup_packages")
            return structuredClone(packages);
          if (command === "inspect_setup_package")
            return structuredClone(packet);
          if (command === "apply_setup_package") {
            packages = [packet];
            populate();
            return null;
          }
          if (command === "desktop_status") return {};
          if (command === "configure_setup_credential") {
            await new Promise<void>((resolve) => {
              host.setupInventory.complete = resolve;
            });
            const p = packet.providers.find(
              (p) => p.profile === args.request.profile,
            )!;
            (p as { credentialId: string | null }).credentialId = "saved-key";
            snapshot.globalConfiguration.credentials.push({
              id: "saved-key",
              label: "Company API key",
              kind: "api_key",
              backend: "vault",
              createdAtMs: 1,
            });
            populate();
            return null;
          }
          if (command === "use_setup_model") {
            await new Promise<void>((resolve) => {
              host.setupInventory.complete = resolve;
            });
            if (host.setupInventory.failActivation)
              throw {
                code: "invalid",
                message: "Matching workspace profiles already exist.",
                violations: [],
                retryable: false,
                outcomeUnknown: false,
              };
            return {};
          }
          if (command === "remove_setup_package") {
            packages = [];
            return null;
          }
          if (
            command === "cancel_setup_package_review" ||
            command === "sync_managed_configuration"
          )
            return null;
          throw new Error(`Unexpected command: ${command}`);
        },
      };
    },
    { sample, imported },
  );
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
}

async function finishNativeAction(page: Page) {
  await page.evaluate(() => (window as any).setupInventory.complete());
}

test("every imported connection is in the normal inventory and setup management stays compact", async ({
  page,
}) => {
  await openInventory(page);
  const rows = page.locator(".catalog-inventory-row");
  await expect(rows).toHaveCount(5);
  await expect(rows.filter({ hasText: "Needs API key" })).toHaveCount(4);
  await expect(rows.filter({ hasText: "Local Models" })).toContainText(
    "No key required",
  );
  expect(
    await page
      .locator(".setup-packages")
      .evaluate((el) => el.getBoundingClientRect().height),
  ).toBeLessThan(170);
  await expect(rows.first()).toBeInViewport();
  await page.getByRole("button", { name: "Manage setup files (1)" }).click();
  const dialog = page.getByRole("dialog", { name: "Setup files" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("5 providers · 6 models")).toBeVisible();
  await expect(dialog.getByRole("table")).toHaveCount(0);
  await dialog.getByText("Remove saved setup", { exact: true }).click();
  await expect(
    dialog.getByText(
      /Providers, models, workspaces, credentials, and trusted certificates are retained/,
    ),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Manage setup files (1)" }),
  ).toBeFocused();
  await page.screenshot({ path: "output/playwright/setup-inventory.png" });
});

test("import refreshes both provider and model inventory immediately without a key", async ({
  page,
}) => {
  await openInventory(page, false);
  await page
    .getByRole("button", { name: "Import setup file", exact: true })
    .click();
  await page.getByRole("button", { name: "Import setup", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".catalog-inventory-row")).toHaveCount(5);
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await expect(page.locator(".catalog-inventory-row")).toHaveCount(6);
  const calls = await page.evaluate(() =>
    (window as any).setupInventory.calls.map((c: any) => c.command),
  );
  expect(calls).not.toContain("configure_setup_credential");
  expect(calls).not.toContain("use_setup_model");
});

test("key enrollment and workspace activation show progress, recover from errors, and refresh", async ({
  page,
}) => {
  await openInventory(page);
  await page
    .getByRole("button", { name: "Details for Company AI", exact: true })
    .click();
  const details = page.getByRole("region", {
    name: "Details for Company AI",
    exact: true,
  });
  const activate = details.getByRole("button", {
    name: "Use model in workspace",
    exact: true,
  });
  await expect(activate).toBeDisabled();
  await details
    .getByRole("button", { name: "Add API key", exact: true })
    .click();
  await expect(details.getByRole("status")).toContainText(
    "Waiting for the secure API key window",
  );
  await expect(details.locator(".setup-action-spinner")).toHaveCSS(
    "animation-name",
    "setup-action-spin",
  );
  await finishNativeAction(page);
  await expect(
    details.getByRole("button", { name: "Change API key", exact: true }),
  ).toBeEnabled();
  await expect(activate).toBeEnabled();
  await activate.click();
  await expect(details.getByRole("status")).toContainText("Applying the model");
  await expect(activate).toBeDisabled();
  await finishNativeAction(page);
  await expect(details.getByRole("alert")).toContainText(
    "Matching workspace profiles",
  );
  await expect(activate).toBeEnabled();
  await details
    .getByRole("checkbox", {
      name: "Replace conflicting profiles in this workspace",
    })
    .check();
  await page.evaluate(() => {
    (window as any).setupInventory.failActivation = false;
  });
  await activate.click();
  await finishNativeAction(page);
  await expect(details.getByRole("status")).toHaveText(
    "Model selected for this workspace.",
  );
  await expect(activate).toBeEnabled();
  const calls = await page.evaluate(() => (window as any).setupInventory.calls);
  expect(
    calls.filter((c: any) => c.command === "use_setup_model").at(-1).args
      .request.replaceConflicts,
  ).toBe(true);
  const result = await new AxeBuilder({ page })
    .include(".catalog-settings")
    .withTags(["wcag2a", "wcag2aa"])
    .analyze();
  expect(result.violations).toEqual([]);
  await page.screenshot({
    path: "output/playwright/setup-inventory-provider-details.png",
  });
});
