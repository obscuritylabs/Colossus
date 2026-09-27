import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
});

for (const [tab, kind, label] of [
  ["Providers", "provider", "primary-provider"],
  ["Models", "model", "primary"],
] as const) {
  test(`${tab} can be searched and inspected with keyboard actions`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.getByRole("button", { name: tab, exact: true }).click();
    const rows = page.locator(".catalog-inventory-row");
    const search = page.getByRole("textbox", {
      name: `Search ${kind}s`,
      exact: true,
    });
    await search.fill(`  ${label.toUpperCase()}  `);
    await expect(rows).toHaveCount(1);
    await search.fill("a-provider-or-model-that-does-not-exist");
    await expect(rows).toHaveCount(0);
    await expect(
      page.getByText(`No matching ${kind}s`, { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Clear search", exact: true })
      .click();
    await expect(search).toBeFocused();
    await expect(rows).toHaveCount(1);
    const details = page.getByRole("region", {
      name: `Details for ${label}`,
      exact: true,
    });
    await expect(details).toBeHidden();
    const usage = rows.getByRole("button", {
      name:
        kind === "provider"
          ? `Configured models for ${label}: 1 model`
          : `Active workspaces for ${label}: 4 workspaces`,
      exact: true,
    });
    await usage.click();
    await expect(usage).toHaveAttribute("aria-expanded", "true");
    await expect(details).toContainText(
      kind === "provider" ? "Configured models" : "Colossus",
    );
    await page
      .getByRole("button", { name: `Details for ${label}`, exact: true })
      .click();
    await expect(details).toBeHidden();

    const actions = page.getByRole("button", {
      name: `More actions for ${label}`,
      exact: true,
    });
    await actions.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: "View details", exact: true }),
    ).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: `Delete ${label}`, exact: true }),
    ).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(actions).toBeFocused();
    await expect(page.getByRole("menu")).toHaveCount(0);
    await actions.click();
    await page
      .getByRole("menuitem", { name: "View details", exact: true })
      .click();
    await expect(details).toBeVisible();
    await expect(actions).toBeFocused();
    await actions.click();
    await search.click();
    await expect(page.getByRole("menu")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}

test("inventory rows reflow without clipped actions in both themes", async ({
  page,
}) => {
  for (const theme of ["Light", "Dark"] as const) {
    await page.getByRole("button", { name: "Desktop", exact: true }).click();
    await page.getByRole("combobox", { name: /^Color theme/u }).click();
    await page.getByRole("option", { name: theme, exact: true }).click();
    for (const width of [1440, 700, 480]) {
      await page.setViewportSize({ width, height: 950 });
      for (const tab of ["Providers", "Models"] as const) {
        await page.getByRole("button", { name: tab, exact: true }).click();
        const inventory = page.locator(".catalog-settings");
        const table = inventory.getByRole("table");
        await expect(table).toBeVisible();
        const bounds = await inventory.evaluate((element) => ({
          client: element.clientWidth,
          scroll: element.scrollWidth,
        }));
        expect(bounds.scroll).toBeLessThanOrEqual(bounds.client + 1);
        const trigger = inventory.getByRole("button", {
          name: /More actions for/u,
        });
        await expect(trigger).toBeInViewport();
        await trigger.click();
        const menuBounds = (await inventory.getByRole("menu").boundingBox())!;
        expect(menuBounds.x).toBeGreaterThanOrEqual(0);
        expect(menuBounds.x + menuBounds.width).toBeLessThanOrEqual(width);
        const accessibility = await new AxeBuilder({ page })
          .include(".catalog-settings")
          .analyze();
        expect(
          accessibility.violations.filter((violation) =>
            ["critical", "serious"].includes(violation.impact ?? ""),
          ),
        ).toEqual([]);
        await page.keyboard.press("Escape");
        await inventory
          .getByRole("heading", { name: tab, exact: true })
          .click();
        await inventory.screenshot({
          path: `output/playwright/inventory-${tab.toLowerCase()}-${theme.toLowerCase()}-${width}.png`,
        });
      }
    }
    await page.setViewportSize({ width: 1440, height: 950 });
  }
});

test("Codex uses a recognizable name while preserving the saved connection label", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  const editor = page.locator(".provider-editor");
  await editor.getByRole("combobox", { name: /^Adapter/u }).click();
  await page
    .getByRole("option", { name: "Codex subscription", exact: true })
    .click();
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  const identity = page.getByRole("button", {
    name: "Details for primary-provider",
    exact: true,
  });
  await expect(identity).toHaveText("Codex");
  await expect(
    page.getByText("primary-provider · Subscription", { exact: true }),
  ).toBeVisible();
  await expect(
    page
      .locator(".catalog-inventory-row")
      .getByText("Codex account", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  await expect(
    editor.getByRole("textbox", { name: /^Display label/u }),
  ).toHaveValue("primary-provider");
  await editor
    .getByRole("textbox", { name: /^Display label/u })
    .fill("Team subscription");
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(
    page.getByRole("button", {
      name: "Details for Team subscription",
      exact: true,
    }),
  ).toHaveText("Team subscription");
});

test("provider branding follows the connection in both inventories and the model editor", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  const editor = page.locator(".provider-editor");
  await editor
    .getByRole("textbox", { name: /^Display label/u })
    .fill("Team gateway");
  await editor
    .getByRole("textbox", { name: /^Endpoint URL/u })
    .fill("https://openrouter.ai/api/v1");
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  const row = page.locator(".catalog-inventory-row");
  await expect(row.locator('[data-provider-brand="openrouter"]')).toBeVisible();
  await expect(
    row.getByRole("button", { name: "Details for Team gateway", exact: true }),
  ).toHaveText("Team gateway");

  for (const theme of ["Light", "Dark"] as const) {
    await page.getByRole("button", { name: "Desktop", exact: true }).click();
    await page.getByRole("combobox", { name: /^Color theme/u }).click();
    await page.getByRole("option", { name: theme, exact: true }).click();
    for (const tab of ["Providers", "Models"] as const) {
      await page.getByRole("button", { name: tab, exact: true }).click();
      const icon = row.locator(
        `[data-provider-brand="openrouter"] .provider-icon-${theme.toLowerCase()}`,
      );
      await expect(icon).toBeVisible();
      await expect
        .poll(() =>
          icon.evaluate((node) => (node as HTMLImageElement).naturalWidth),
        )
        .toBeGreaterThan(0);
      await page.locator(".catalog-settings").screenshot({
        path: `output/playwright/provider-brand-${tab.toLowerCase()}-${theme.toLowerCase()}.png`,
      });
    }
  }
  await page.getByRole("button", { name: "Edit primary", exact: true }).click();
  const savedProvider = page.getByRole("combobox", {
    name: /^Provider profile/u,
  });
  await expect(
    savedProvider.locator('[data-provider-brand="openrouter"]'),
  ).toBeVisible();
  await page
    .locator(".model-editor")
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit Team gateway", exact: true })
    .click();
  await editor.getByRole("textbox", { name: /^Display label/u }).fill("OpenAI");
  await editor
    .getByRole("textbox", { name: /^Endpoint URL/u })
    .fill("https://gateway.example.test/v1");
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(row.locator('[data-provider-brand="custom"]')).toBeVisible();
  await expect(row.locator(".provider-icon img")).toHaveCount(0);
});
