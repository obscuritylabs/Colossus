import { expect, test, type Page } from "@playwright/test";

async function openGlobalSettings(page: Page, section: string) {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: section, exact: true }).click();
}

for (const entry of [
  { section: "Providers", label: "primary-provider", kind: "provider" },
  { section: "Models", label: "primary", kind: "model" },
]) {
  test(`${entry.kind} save failure retains the draft and allows a successful retry`, async ({
    page,
  }) => {
    await openGlobalSettings(page, entry.section);
    await page
      .getByRole("button", { name: `Edit ${entry.label}`, exact: true })
      .click();
    const editor = page.locator(`.${entry.kind}-editor`);
    const label = editor.getByRole("textbox", {
      name: "Display label",
      exact: false,
    });
    await label.fill(`Recovered ${entry.kind}`);
    await page.evaluate(() => {
      const state = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        settingsRequests: { command: string; args: unknown }[];
        rejectSettingsSave: () => void;
      };
      state.settingsRequests = [];
      state.__TAURI_INTERNALS__ = {
        invoke: (command: string, args: unknown) => {
          state.settingsRequests.push({ command, args });
          return new Promise((_resolve, reject) => {
            state.rejectSettingsSave = () =>
              reject({
                code: "invalid_argument",
                retryable: true,
                outcomeUnknown: false,
                message: "The settings change failed.",
                violations: [
                  {
                    field: "profile",
                    description:
                      "This profile could not be saved. Review it and retry.",
                  },
                ],
              });
          });
        },
      };
    });
    await editor
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(
      editor.getByRole("button", { name: "Save changes", exact: true }),
    ).toBeDisabled();
    await expect(
      editor.getByRole("button", { name: "Cancel", exact: true }),
    ).toBeDisabled();
    await expect(label).toBeDisabled();
    await expect(
      page.getByRole("button", { name: `Edit ${entry.label}`, exact: true }),
    ).toBeDisabled();
    await page.evaluate(() =>
      (
        window as unknown as { rejectSettingsSave: () => void }
      ).rejectSettingsSave(),
    );
    await expect(page.getByRole("alert")).toContainText("Review it and retry.");
    await expect(editor).toBeVisible();
    await expect(label).toHaveValue(`Recovered ${entry.kind}`);
    await expect(label).toBeEnabled();
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { settingsRequests: unknown[] })
            .settingsRequests,
      ),
    ).toEqual([
      expect.objectContaining({ command: `upsert_global_${entry.kind}` }),
    ]);
    // The fixture applies the same submitted draft after the native failure.
    await page.evaluate(() => {
      delete (window as unknown as { __TAURI_INTERNALS__?: unknown })
        .__TAURI_INTERNALS__;
    });
    await editor
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(editor).toHaveCount(0);
    await expect(
      page.getByRole("button", {
        name: `Edit Recovered ${entry.kind}`,
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
  });
}

test("model discovery holds settings busy until native completion even after changing tabs", async ({
  page,
}) => {
  await openGlobalSettings(page, "Models");
  await page.getByRole("button", { name: "Edit primary", exact: true }).click();
  await page.evaluate(() => {
    const state = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      settingsRequests: { command: string; args: unknown }[];
      finishSettingsCatalog: () => void;
    };
    state.settingsRequests = [];
    state.__TAURI_INTERNALS__ = {
      invoke: (command: string, args: unknown) => {
        state.settingsRequests.push({ command, args });
        return new Promise((resolve) => {
          state.finishSettingsCatalog = () =>
            resolve({
              credentialId: null,
              models: [
                {
                  id: "stale/catalog-model",
                  display_name: "Stale catalog model",
                },
              ],
            });
        });
      },
    };
  });
  const editor = page.locator(".model-editor");
  await editor
    .getByRole("button", { name: "Load models", exact: true })
    .click();
  await expect(
    editor.getByRole("button", { name: "Loading models…", exact: true }),
  ).toBeDisabled();
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await expect(
    editor.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeDisabled();
  await expect(
    editor.getByRole("textbox", { name: "Model identifier", exact: false }),
  ).toBeDisabled();
  await expect(
    editor.getByRole("combobox", { name: "Provider profile", exact: false }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Add provider", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Edit primary-provider", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { settingsRequests: unknown[] }).settingsRequests,
    ),
  ).toEqual([
    expect.objectContaining({ command: "discover_managed_provider_models" }),
  ]);
  await page.evaluate(() =>
    (
      window as unknown as { finishSettingsCatalog: () => void }
    ).finishSettingsCatalog(),
  );
  await expect(
    page.getByRole("button", { name: "Add provider", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await expect(
    editor.getByRole("button", { name: "Load models", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByText("Stale catalog model", { exact: true }),
  ).toHaveCount(0);
  await editor
    .getByRole("button", { name: "Load models", exact: true })
    .click();
  await page.evaluate(() =>
    (
      window as unknown as { finishSettingsCatalog: () => void }
    ).finishSettingsCatalog(),
  );
  const modelCard = editor.getByRole("button", { name: /Stale catalog model/ });
  await expect(modelCard).toBeEnabled();
  await modelCard.click();
  await expect(
    editor.getByRole("textbox", { name: "Model identifier", exact: false }),
  ).toHaveValue("stale/catalog-model");
});

test("catalog failure preserves the model editor and a retry imports only advertised metadata", async ({
  page,
}) => {
  await openGlobalSettings(page, "Models");
  await page.getByRole("button", { name: "Edit primary", exact: true }).click();
  const editor = page.locator(".model-editor");
  const identifier = editor.getByRole("textbox", {
    name: "Model identifier",
    exact: false,
  });
  const originalModel = await identifier.inputValue();
  await page.evaluate(() => {
    const state = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      failSettingsCatalog: boolean;
    };
    state.failSettingsCatalog = true;
    state.__TAURI_INTERNALS__ = {
      invoke: async () =>
        state.failSettingsCatalog
          ? {
              models: [],
              credentialId: null,
              errorMessage: "Catalog authentication failed.",
            }
          : { models: [{ id: "catalog/plain-model" }], credentialId: null },
    };
  });
  await editor
    .getByRole("button", { name: "Load models", exact: true })
    .click();
  await expect(editor.getByRole("alert")).toContainText(
    "Catalog authentication failed.",
  );
  await expect(identifier).toHaveValue(originalModel);
  await expect(identifier).toBeEnabled();
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() => {
    (
      window as unknown as { failSettingsCatalog: boolean }
    ).failSettingsCatalog = false;
  });
  await editor
    .getByRole("button", { name: "Retry loading models", exact: true })
    .click();
  await editor.getByRole("button", { name: /catalog\/plain-model/ }).click();
  await expect(identifier).toHaveValue("catalog/plain-model");
  await expect(
    editor.getByRole("spinbutton", {
      name: "Context window (tokens)",
      exact: false,
    }),
  ).toHaveValue("32768");
  await expect(
    editor.getByRole("spinbutton", {
      name: "Maximum output (tokens)",
      exact: false,
    }),
  ).toHaveValue("4096");
  await expect(
    editor.getByRole("switch", { name: /Tool calls/ }),
  ).not.toBeChecked();
  await expect(
    editor.getByRole("switch", { name: /Streaming/ }),
  ).not.toBeChecked();
});
