import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
});

test("model settings opens the workspace route and preserves an unsent draft", async ({
  page,
}) => {
  const draft = "Check the composer without sending this message.";
  await page.getByRole("textbox", { name: "Prompt", exact: true }).fill(draft);
  const model = page.getByRole("button", {
    name: "Model settings: fixture · OpenRouter",
    exact: true,
  });
  await expect(
    model.locator('[data-provider-brand="openrouter"]'),
  ).toBeVisible();
  await model.click();
  await expect(
    page.getByRole("heading", { name: "Providers and models", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Workspace", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("button", { name: "Providers", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue(draft);
  await model.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Providers and models", exact: true }),
  ).toBeVisible();
});

test("composer retains visible, accessible controls at narrow widths in both themes", async ({
  page,
}) => {
  // The wide fixture starts with thread details open. Close that panel before
  // narrowing the viewport, where it deliberately becomes a modal overlay.
  await page
    .getByRole("button", { name: "Close thread details", exact: true })
    .click();
  for (const theme of ["light", "dark"]) {
    await page.evaluate(
      (theme) => document.documentElement.setAttribute("data-theme", theme),
      theme,
    );
    for (const width of [1440, 880, 480]) {
      await page.setViewportSize({ width, height: 950 });
      const composer = page.getByRole("form", {
        name: "Send a prompt",
        exact: true,
      });
      await composer
        .getByRole("textbox", { name: "Prompt", exact: true })
        .click();
      await expect(
        composer.getByRole("textbox", { name: "Prompt", exact: true }),
      ).toBeFocused();
      await expect(
        composer.getByRole("button", { name: /^Model settings:/ }),
      ).toBeInViewport();
      await expect(
        composer.getByRole("combobox", {
          name: "Permission mode",
          exact: true,
        }),
      ).toBeInViewport();
      await expect(
        composer.getByRole("button", {
          name: "Add message to Next up",
          exact: true,
        }),
      ).toBeInViewport();
      const bounds = await composer.evaluate((element) => ({
        client: element.clientWidth,
        scroll: element.scrollWidth,
      }));
      expect(bounds.scroll).toBeLessThanOrEqual(bounds.client + 1);
      const sections = await composer.evaluate((element) => {
        const rect = (selector: string) =>
          element.querySelector(selector)!.getBoundingClientRect();
        return {
          headerBottom: rect(".composer-header").bottom,
          messageTop: rect("textarea").top,
          messageBottom: rect("textarea").bottom,
          footerTop: rect(".composer-footer").top,
        };
      });
      expect(sections.headerBottom).toBeLessThanOrEqual(sections.messageTop);
      expect(sections.messageBottom).toBeLessThanOrEqual(sections.footerTop);
      await expect(composer.locator(".composer-meta")).not.toContainText(
        "bytes",
      );
      const result = await new AxeBuilder({ page })
        .include(".work-composer")
        .analyze();
      expect(
        result.violations.filter((violation) =>
          ["critical", "serious"].includes(violation.impact ?? ""),
        ),
      ).toEqual([]);
      await composer.screenshot({
        path: `output/composer-review/composer-${theme}-${width}.png`,
      });
    }
  }
});

test("new messages use a compact header, writing area, and footer", async ({
  page,
}) => {
  await page.setViewportSize({ width: 880, height: 950 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page.getByRole("button", { name: "New thread in Colossus" }).click();
  const composer = page.getByRole("form", {
    name: "Send a prompt",
    exact: true,
  });
  const prompt = composer.getByRole("textbox", { name: "Prompt", exact: true });
  for (const width of [1440, 480]) {
    await page.setViewportSize({ width, height: 950 });
    await prompt.fill("Review this change");
    await prompt.press("Shift+Enter");
    await expect(prompt).toHaveValue("Review this change\n");
    await expect(
      composer.getByRole("button", { name: "Send prompt", exact: true }),
    ).toBeInViewport();
    await expect(composer.locator(".composer-footer")).toContainText(
      "Enter to send · Shift+Enter for a new line",
    );
    const bounds = await composer.boundingBox();
    expect(bounds!.height).toBeLessThanOrEqual(width > 700 ? 160 : 190);
    await prompt.clear();
    await composer.screenshot({
      path: `output/composer-review/composer-header-footer-${width}.png`,
    });
  }
});
