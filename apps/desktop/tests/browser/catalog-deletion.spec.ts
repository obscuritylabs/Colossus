import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 850 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
});

async function openDelete(page: Page, label: string) {
  await page
    .getByRole("button", { name: `More actions for ${label}`, exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: `Delete ${label}`, exact: true })
    .click();
}

async function addProvider(page: Page) {
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const editor = page.locator(".provider-editor");
  await editor
    .getByRole("textbox", { name: "Display label", exact: false })
    .fill("Temporary provider");
  await editor
    .getByRole("textbox", { name: "Profile ID", exact: false })
    .fill("temporary-provider");
  await editor
    .getByRole("textbox", { name: "Endpoint URL", exact: false })
    .fill("https://example.test/v1");
  await editor
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
}

async function addModel(page: Page, provider?: string) {
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByRole("button", { name: "Add model", exact: true }).click();
  const editor = page.locator(".model-editor");
  await editor
    .getByRole("textbox", { name: "Display label", exact: false })
    .fill("Temporary model");
  await editor
    .getByRole("textbox", { name: "Profile ID", exact: false })
    .fill("temporary-model");
  if (provider) {
    await editor
      .getByRole("combobox", { name: "Provider profile", exact: false })
      .click();
    await page.getByRole("option", { name: provider, exact: true }).click();
  }
  await editor
    .getByRole("textbox", { name: "Model identifier", exact: false })
    .fill("example-model");
  await editor.getByRole("button", { name: "Add model", exact: true }).click();
  await expect(editor).toHaveCount(0);
}

test("an unused model can be deleted, then its provider can be deleted without losing credentials", async ({
  page,
}) => {
  await addProvider(page);
  await addModel(page, "Temporary provider");
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await openDelete(page, "Temporary provider");
  let dialog = page.getByRole("dialog", { name: "Delete Temporary provider?" });
  await expect(dialog.getByRole("listitem")).toHaveText([
    "Model Temporary model",
  ]);
  await expect(
    dialog.getByRole("button", { name: "Delete provider", exact: true }),
  ).toBeDisabled();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit Temporary model", exact: true })
    .click();
  await openDelete(page, "Temporary model");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Delete model", exact: true })
    .click();
  await expect(page.locator(".model-editor")).toHaveCount(0);
  await expect(
    page.getByRole("button", {
      name: "More actions for Temporary model",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(page.locator("#add-model")).toBeFocused();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await openDelete(page, "Temporary provider");
  dialog = page.getByRole("dialog", { name: "Delete Temporary provider?" });
  await expect(dialog).toContainText(
    "Saved credentials and your provider account will be kept.",
  );
  await dialog
    .getByRole("button", { name: "Delete provider", exact: true })
    .click();
  await expect(
    page.getByRole("button", {
      name: "More actions for Temporary provider",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(page.locator("#add-provider")).toBeFocused();
  await page.getByRole("button", { name: "Credentials", exact: true }).click();
  await expect(
    page.getByText("GitHub workspace token", { exact: true }),
  ).toBeVisible();
});

for (const kind of ["model", "provider"] as const) {
  test(`${kind} deletion cancels, blocks duplicate requests, and preserves the row after a native failure`, async ({
    page,
  }) => {
    if (kind === "model") await addModel(page);
    else await addProvider(page);
    const trigger = page.getByRole("button", {
      name: `More actions for Temporary ${kind}`,
      exact: true,
    });
    await openDelete(page, `Temporary ${kind}`);
    const dialog = page.getByRole("dialog", {
      name: `Delete Temporary ${kind}?`,
    });
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
    await openDelete(page, `Temporary ${kind}`);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(trigger).toBeFocused();
    await openDelete(page, `Temporary ${kind}`);
    await page.evaluate(() => {
      const state = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        deletionCalls: unknown[];
        rejectDeletion: () => void;
      };
      state.deletionCalls = [];
      state.__TAURI_INTERNALS__ = {
        invoke: (command: string, args: unknown) => {
          state.deletionCalls.push({ command, args });
          return new Promise((_resolve, reject) => {
            state.rejectDeletion = () =>
              reject({
                code: "busy",
                message:
                  "Global settings changed in another window. Reload and review the latest revision.",
                retryable: true,
                outcomeUnknown: false,
                violations: [],
              });
          });
        },
      };
    });
    await dialog
      .getByRole("button", { name: `Delete ${kind}`, exact: true })
      .click();
    await expect(dialog.getByRole("status")).toHaveText(`Deleting ${kind}…`);
    await expect(
      dialog.getByRole("button", { name: `Delete ${kind}`, exact: true }),
    ).toBeDisabled();
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeVisible();
    expect(
      await page.evaluate(
        () => (window as unknown as { deletionCalls: unknown[] }).deletionCalls,
      ),
    ).toEqual([
      {
        command: `delete_global_${kind}`,
        args: {
          request: { expectedRevision: 5, resourceId: expect.any(String) },
        },
      },
    ]);
    await page.evaluate(() =>
      (window as unknown as { rejectDeletion: () => void }).rejectDeletion(),
    );
    await expect(dialog.getByRole("alert")).toContainText(
      "Reload and review the latest revision.",
    );
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(trigger).toBeVisible();
    await page.evaluate(() => {
      delete (window as unknown as { __TAURI_INTERNALS__?: unknown })
        .__TAURI_INTERNALS__;
    });
    await openDelete(page, `Temporary ${kind}`);
    await dialog
      .getByRole("button", { name: `Delete ${kind}`, exact: true })
      .click();
    await expect(trigger).toHaveCount(0);
  });

  test(`in-use ${kind} deletion explains dependencies at a compact width`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 700, height: 800 });
    await page
      .getByRole("button", {
        name: kind === "model" ? "Models" : "Providers",
        exact: true,
      })
      .click();
    const trigger = page.getByRole("button", {
      name: `More actions for ${kind === "model" ? "primary" : "primary-provider"}`,
      exact: true,
    });
    await expect(trigger).toBeInViewport();
    const box = (await trigger.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(700);
    await openDelete(page, kind === "model" ? "primary" : "primary-provider");
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("Workspace Colossus");
    await expect(
      dialog.getByRole("button", { name: `Delete ${kind}`, exact: true }),
    ).toBeDisabled();
    const accessibility = await new AxeBuilder({ page })
      .include(".catalog-delete-dialog")
      .analyze();
    expect(
      accessibility.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
    ).toEqual([]);
    await dialog.screenshot({
      path: `output/playwright/${kind}-delete-blocked.png`,
    });
  });
}
