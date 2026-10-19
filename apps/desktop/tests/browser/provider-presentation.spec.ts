import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const png = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAGUlEQVR4nGPQqH32nxLMMGrAqAGjBgwXAwAqWYofjPA1NQAAAABJRU5ErkJggg==",
  "base64",
);

test("OpenRouter retains its API-key requirement and can add a credential directly", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("button", { name: "Settings", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    (
      window as unknown as { __TAURI_INTERNALS__: unknown }
    ).__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        if (command === "list_setup_packages") return [];
        if (command === "get_provider_presets")
          return [
            {
              id: "openrouter",
              label: "OpenRouter",
              protocol: "chat_completions",
              baseUrl: "https://openrouter.ai/api/v1",
              credentialEnv: "OPENROUTER_API_KEY",
            },
          ];
        throw new Error("Fixture command unavailable");
      },
    };
  });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const editor = page.locator(".provider-editor");
  await editor.getByRole("combobox", { name: "Provider", exact: true }).click();
  await page.getByRole("option", { name: "OpenRouter", exact: true }).click();
  // The catalog comes from native metadata; settings writes use the local fixture.
  await page.evaluate(() => {
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__;
  });
  await expect(
    editor.getByRole("combobox", { name: "Credential reference" }),
  ).toContainText("Add API key later");
  await expect(editor).not.toContainText("No API key required");
  const addKey = editor.getByRole("button", { name: "Add API key securely" });
  await addKey.focus();
  await expect(addKey).toBeFocused();
  await addKey.press("Enter");
  await expect(
    editor.getByRole("combobox", { name: "Credential reference" }),
  ).toContainText("OpenRouter API key");
  await expect(editor.locator('input[type="password"]')).toHaveCount(0);
  await page.screenshot({ path: "output/playwright/openrouter-api-key.png" });
  for (const [theme, palette, width] of [
    ["light", "colossus", 1440],
    ["dark", "colossus", 880],
    ["dark", "neutral", 620],
    ["dark", "hacker", 620],
  ] as const) {
    await page.setViewportSize({ width, height: 1050 });
    await page.evaluate(
      ({ theme, palette }) => {
        document.documentElement.dataset.theme = theme;
        document.documentElement.dataset.palette = palette;
        document.documentElement.dataset.textSize = "large";
      },
      { theme, palette },
    );
    const authentication = editor.locator(".provider-auth-grid");
    expect(
      await authentication.evaluate((el) => el.scrollWidth <= el.clientWidth),
    ).toBe(true);
    const accessibility = await new AxeBuilder({ page })
      .include(".provider-auth-grid")
      .analyze();
    expect(accessibility.violations).toEqual([]);
    await authentication.screenshot({
      path: `output/playwright/openrouter-auth-${theme}-${palette}.png`,
    });
  }
  await editor
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Edit OpenRouter", exact: true }),
  ).toBeVisible();
});

test("provider instructions and embedded icons can be authored, previewed, saved, and cleared", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  const editor = page.locator(".provider-editor");
  await editor.getByText("Advanced options", { exact: false }).click();
  const description = editor.getByRole("textbox", {
    name: "Description & instructions Markdown",
  });
  const instructions =
    "### Get a token\nVisit [Team portal](https://example.test).\n\n- Request access from your administrator.";
  await description.fill(instructions);
  await editor.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(
    editor.getByRole("heading", { name: "Get a token" }),
  ).toBeVisible();
  await editor
    .getByRole("button", { name: "Edit Markdown", exact: true })
    .click();
  await expect(description).toHaveValue(instructions);
  const upload = editor.getByLabel("Upload provider icon", { exact: true });
  await upload.setInputFiles({
    name: "team.svg",
    mimeType: "image/svg+xml",
    buffer: Buffer.from("<svg/>"),
  });
  await expect(editor.getByRole("alert")).toHaveText("Choose a PNG image.");
  const oversized = Buffer.from(png);
  oversized.writeUInt32BE(4096, 16);
  await upload.setInputFiles({
    name: "large.png",
    mimeType: "image/png",
    buffer: oversized,
  });
  await expect(editor.getByRole("alert")).toContainText("512");
  await upload.setInputFiles({
    name: "team.png",
    mimeType: "image/png",
    buffer: png,
  });
  await expect(
    editor.getByRole("button", { name: "Remove provider icon" }),
  ).toBeVisible();
  await expect(editor.getByRole("alert")).toHaveCount(0);
  await editor
    .getByLabel("Upload dark theme icon")
    .setInputFiles({ name: "dark.png", mimeType: "image/png", buffer: png });
  await expect(
    editor.getByRole("button", { name: "Remove dark theme icon" }),
  ).toBeVisible();
  const presentation = editor.locator(".provider-advanced-options");
  const accessibility = await new AxeBuilder({ page })
    .include(".provider-advanced-options")
    .analyze();
  expect(
    accessibility.violations.filter((v) =>
      ["critical", "serious"].includes(v.impact ?? ""),
    ),
  ).toEqual([]);
  for (const width of [1440, 700, 480]) {
    await page.setViewportSize({ width, height: 1050 });
    const bounds = await presentation.evaluate((el) => ({
      client: el.clientWidth,
      scroll: el.scrollWidth,
    }));
    expect(bounds.scroll).toBeLessThanOrEqual(bounds.client + 1);
  }
  await page.setViewportSize({ width: 1440, height: 1050 });
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  await page
    .getByRole("button", { name: "Details for primary-provider", exact: true })
    .click();
  await expect(
    page
      .getByRole("region", { name: "Provider instructions" })
      .getByRole("heading", { name: "Get a token" }),
  ).toBeVisible();
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      instructionLink: unknown;
    };
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: { request?: unknown }) => {
        if (command === "open_setup_link") {
          host.instructionLink = args.request;
          return;
        }
        throw new Error("Native command is unavailable in this fixture");
      },
    };
  });
  await page
    .getByRole("button", { name: "Team portal (open in browser)", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { instructionLink: unknown }).instructionLink,
      ),
    )
    .toMatchObject({ url: "https://example.test" });
  await page.evaluate(() => {
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__;
  });
  const row = page.locator(".catalog-inventory-row");
  await expect(row.locator('img[src^="data:image/png;base64,"]')).toHaveCount(
    2,
  );
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  await editor.getByText("Advanced options", { exact: false }).click();
  await expect(description).toHaveValue(instructions);
  await description.fill("x".repeat(16385));
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await description.clear();
  await editor
    .getByRole("button", { name: "Remove provider icon", exact: true })
    .click();
  await editor
    .getByRole("button", { name: "Remove dark theme icon", exact: true })
    .click();
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  await expect(
    page.getByRole("region", { name: "Provider instructions" }),
  ).toHaveCount(0);
  await expect(row.locator('img[src^="data:image/png;base64,"]')).toHaveCount(
    0,
  );
  expect(errors).toEqual([]);
});
