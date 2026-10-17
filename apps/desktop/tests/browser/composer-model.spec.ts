import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
});

test("model and reasoning pickers preserve the draft and reset reasoning on a model change", async ({
  page,
}) => {
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Keep this draft while I choose a model.");
  const model = page.getByRole("button", {
    name: "Choose model and reasoning: fixture",
    exact: true,
  });
  const effort = page.getByRole("slider", {
    name: "Reasoning effort for next message",
    exact: true,
  });
  await model.click();
  await page
    .getByRole("combobox", { name: "Search models" })
    .fill("primary-provider");
  await page
    .getByRole("option", {
      name: "fixture OpenRouter · primary (primary-provider)",
      exact: true,
    })
    .click();
  await expect(effort).toBeFocused();
  await effort.press("Home");
  for (let step = 0; step < 5; step++) await effort.press("ArrowRight");
  await expect(effort).toHaveAttribute("aria-valuetext", "high");
  await expect(prompt).toHaveValue("Keep this draft while I choose a model.");
  await page
    .getByRole("option", {
      name: "fixture OpenRouter · primary (primary-provider)",
      exact: true,
    })
    .click();
  await expect(effort).toHaveAttribute("aria-valuetext", "high");
  await page
    .getByRole("option", {
      name: "Role default fixture · OpenRouter",
      exact: true,
    })
    .click();
  await expect(effort).toHaveAttribute("aria-valuetext", "Model default");
  await effort.press("Escape");
  await expect(model).toBeFocused();
  await model.press("Enter");
  const search = page.getByRole("combobox", { name: "Search models" });
  await search.fill("unconfigured-model");
  await expect(
    page.getByRole("status").filter({ hasText: "No matching models" }),
  ).toBeVisible();
  await search.press("Enter");
  await expect(prompt).toHaveValue("Keep this draft while I choose a model.");
  await search.press("Escape");
});

test("model settings opens the workspace route and preserves an unsent draft", async ({
  page,
}) => {
  const draft = "Check the composer without sending this message.";
  await page.getByRole("textbox", { name: "Prompt", exact: true }).fill(draft);
  const model = page.getByRole("button", {
    name: "Choose model and reasoning: fixture",
    exact: true,
  });
  await expect(
    page.locator('.composer-model-controls [data-provider-brand="openrouter"]'),
  ).toBeVisible();
  await model.click();
  await page
    .getByRole("button", { name: "Configure models", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Models", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Workspace", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("button", { name: "Models", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue(draft);
  await model.press("Enter");
  await page
    .getByRole("button", { name: "Configure models", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Models", exact: true }),
  ).toBeVisible();
});

test("a new thread starts with role defaults after another thread used overrides", async ({
  page,
}) => {
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await page.setViewportSize({ width: 880, height: 950 });
  async function newThread() {
    await page.getByRole("button", { name: "Open work navigation" }).click();
    await page.getByRole("button", { name: "New thread in Colossus" }).click();
  }
  await newThread();
  const model = page.getByRole("button", {
    name: "Choose model and reasoning: fixture",
    exact: true,
  });
  await model.click();
  await page
    .getByRole("option", {
      name: "fixture OpenRouter · primary (primary-provider)",
      exact: true,
    })
    .click();
  const slider = page.getByRole("slider", {
    name: "Reasoning effort for next message",
  });
  await slider.press("End");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Prompt", exact: true })
    .fill("Remember this thread's selected model");
  await page.getByRole("button", { name: "Send prompt", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue("");
  await expect(model).toContainText("ultra");
  await newThread();
  await model.click();
  await expect(
    page.getByRole("option", {
      name: "Role default fixture · OpenRouter",
      exact: true,
    }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(slider).toHaveAttribute("aria-valuetext", "Model default");
});

test("browser conversation keeps model and reasoning after its first message", async ({
  page,
}) => {
  // Supply only the BrowserApi response; the real App handoff and submission
  // state run in the existing renderer fixture without a native browser.
  const snapshot = {
    available: true,
    generation: 1,
    tabs: [
      {
        id: "model-handoff-tab",
        url: "https://example.test/",
        title: "Model handoff page",
        canGoBack: false,
        canGoForward: false,
        loading: false,
        error: null,
        notice: null,
        popupUrl: null,
        control: "human",
        conversationId: "model-handoff-conversation",
      },
    ],
    selectedTabId: "model-handoff-tab",
    engine: {
      kind: "owned_chromium",
      preview: false,
      ready: true,
      message: null,
      agentControlAvailable: true,
    },
  };
  await page.route("**/src/dev/browser-fixture.ts", (route) =>
    route.fulfill({
      contentType: "text/javascript",
      body: `export function browserFixture() {
        const snapshot = ${JSON.stringify(snapshot)};
        return {
          context: async () => structuredClone(snapshot),
          command: async () => structuredClone(snapshot),
          viewport: async () => {},
        };
      }`,
    }),
  );
  await page.reload();
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
  await page
    .getByRole("button", { name: "Use in a new conversation", exact: true })
    .click();
  const model = page.getByRole("button", {
    name: "Choose model and reasoning: fixture",
    exact: true,
  });
  const explicitModel = page.getByRole("option", {
    name: "fixture OpenRouter · primary (primary-provider)",
    exact: true,
  });
  const slider = page.getByRole("slider", {
    name: "Reasoning effort for next message",
    exact: true,
  });
  await model.click();
  await explicitModel.click();
  await slider.press("End");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  for (const message of [
    "Use this browser page",
    "Continue with the same model",
  ]) {
    await prompt.fill(message);
    await page
      .getByRole("button", { name: "Send prompt", exact: true })
      .click();
    await expect(prompt).toHaveValue("");
    await expect(model).toContainText("ultra");
    await expect(
      page.getByRole("button", { name: "Give agent control", exact: true }),
    ).toBeVisible();
    await model.click();
    await expect(explicitModel).toHaveAttribute("aria-selected", "true");
    await expect(slider).toHaveAttribute("aria-valuetext", "ultra");
    await page.getByRole("button", { name: "Done", exact: true }).click();
  }
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await page.setViewportSize({ width: 880, height: 950 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page.getByRole("button", { name: "New thread in Colossus" }).click();
  await model.click();
  await expect(
    page.getByRole("option", {
      name: "Role default fixture · OpenRouter",
      exact: true,
    }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(slider).toHaveAttribute("aria-valuetext", "Model default");
});

test("composer retains visible, accessible controls at narrow widths in both themes", async ({
  page,
}) => {
  // The wide fixture starts with thread details open. Close that panel before
  // narrowing the viewport, where it deliberately becomes a modal overlay.
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
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
        composer.getByRole("button", { name: /^Choose model and reasoning:/ }),
      ).toBeInViewport();
      await expect(
        composer.getByRole("button", { name: /^Open Git:/ }),
      ).toBeInViewport();
      await expect(
        composer.getByRole("combobox", {
          name: "Permission mode",
          exact: true,
        }),
      ).toBeInViewport();
      await expect(
        composer.getByRole("button", {
          name: "Stop response",
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
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
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

test("model popover supports palettes, large text, narrow screens and keyboard reasoning", async ({
  page,
}) => {
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await page.setViewportSize({ width: 480, height: 800 });
  for (const appearance of [
    { theme: "light", palette: "neutral", size: "compact", contrast: false },
    { theme: "dark", palette: "neutral", size: "comfortable", contrast: false },
    { theme: "dark", palette: "hacker", size: "large", contrast: false },
    { theme: "dark", palette: "black", size: "large", contrast: true },
  ]) {
    await page.emulateMedia({
      forcedColors: appearance.contrast ? "active" : "none",
    });
    await page.evaluate(({ theme, palette, size }) => {
      document.documentElement.setAttribute("data-theme", theme);
      document.documentElement.setAttribute("data-palette", palette);
      document.documentElement.setAttribute("data-text-size", size);
    }, appearance);
    const trigger = page.getByRole("button", {
      name: "Choose model and reasoning: fixture",
      exact: true,
    });
    await trigger.click();
    const picker = page.getByRole("dialog", {
      name: "Model and reasoning",
      exact: true,
    });
    await expect(
      page.getByRole("combobox", { name: "Search models" }),
    ).toBeFocused();
    await page
      .getByRole("combobox", { name: "Search models" })
      .press("ArrowDown");
    await page.getByRole("combobox", { name: "Search models" }).press("Enter");
    const slider = picker.getByRole("slider", {
      name: "Reasoning effort for next message",
    });
    await expect(slider).toBeFocused();
    await slider.press("End");
    await expect(slider).toHaveAttribute("aria-valuetext", "ultra");
    await slider.press("ArrowLeft");
    await expect(slider).toHaveAttribute("aria-valuetext", "max");
    await slider.press("Home");
    await expect(slider).toHaveAttribute("aria-valuetext", "Model default");
    await expect(slider).toBeInViewport();
    for (const icon of await picker.locator(".provider-icon").all()) {
      const iconBounds = await icon.boundingBox();
      expect(iconBounds!.width).toBeLessThanOrEqual(18);
    }
    await expect(
      picker.getByRole("button", { name: "Done", exact: true }),
    ).toBeInViewport();
    const bounds = await picker.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(480);
    const result = await new AxeBuilder({ page })
      .include(".ui-model-picker")
      .analyze();
    expect(
      result.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
    ).toEqual([]);
    await picker.screenshot({
      path: `output/composer-review/model-picker-${appearance.theme}-${appearance.palette}-${appearance.size}-${appearance.contrast}.png`,
    });
    await slider.press("Escape");
    await expect(trigger).toBeFocused();
  }
});
