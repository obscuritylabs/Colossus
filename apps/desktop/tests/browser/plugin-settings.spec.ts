import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.use({ colorScheme: "dark" });

for (const width of [1586, 700]) {
  test(`plugin settings are reachable, editable, and scoped at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 990 });
    await page.goto("/?fixture=operations-studio");
    if (width < 980) {
      await page.getByRole("button", { name: "Open work navigation" }).click();
    }
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const sidebar = page.getByRole("complementary", {
      name: "Settings navigation",
    });
    const sections = sidebar.getByRole("navigation", {
      name: "Settings sections",
    });
    const plugins = sections.getByRole("button", {
      name: "Plugins",
      exact: true,
    });
    const search = page.getByRole("searchbox", { name: "Search settings" });
    const discard = page.getByRole("button", { name: "Discard", exact: true });
    const globalSave = page.getByRole("button", {
      name: "Save global changes",
    });
    const workspaceSave = page.getByRole("button", {
      name: "Apply Workspace changes",
    });

    await sidebar.getByRole("button", { name: "Global", exact: true }).click();
    await plugins.focus();
    await page.keyboard.press("Enter");
    await expect(
      page.getByRole("heading", { name: "Plugins", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("checkbox", { name: "Agent Plugins", exact: true }),
    ).toBeChecked();
    await expect(globalSave).toBeDisabled();
    await page
      .getByRole("textbox", { name: "Included plugins", exact: true })
      .fill("colossus");
    await page.getByLabel("New plugins.registries key").fill("internal");
    await page
      .getByRole("button", { name: "Add plugins.registries entry" })
      .click();
    await page
      .getByLabel("Exact registry origin")
      .fill("https://registry.example");
    await page
      .getByLabel("Registry CA bundle path")
      .fill("C:\\certificates\\company.pem");
    await page.getByLabel("New plugins.mcpServers key").fill("example/server");
    await page
      .getByRole("button", { name: "Add plugins.mcpServers entry" })
      .click();
    await page
      .getByLabel("Allowed tool names (or a sole *)")
      .fill("search\nread");
    await expect(
      page.getByLabel("Explicitly enable this MCP server"),
    ).not.toBeChecked();
    await expect(
      page.getByRole("combobox", { name: "Signature policy" }),
    ).toHaveText("required");

    const layout = await page
      .locator(".plugin-settings")
      .evaluate((element) => {
        const body = element.getBoundingClientRect();
        const outside = [
          ...element.querySelectorAll("input, textarea, button, fieldset"),
        ]
          .filter((control) => {
            const bounds = control.getBoundingClientRect();
            return (
              bounds.width > 0 &&
              (bounds.left < body.left || bounds.right > body.right + 1)
            );
          })
          .map((control) => control.outerHTML.slice(0, 100));
        return { overflow: element.scrollWidth - element.clientWidth, outside };
      });
    expect(layout).toEqual({ overflow: 0, outside: [] });
    const accessibility = await new AxeBuilder({ page })
      .include(".managed-settings-shell")
      .analyze();
    expect(
      accessibility.violations.filter(({ impact }) =>
        ["critical", "serious"].includes(impact ?? ""),
      ),
    ).toEqual([]);
    await page.locator(".settings-main").evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.screenshot({
      path: `output/playwright/plugin-settings-${width}.png`,
    });
    await page
      .locator('[id="managed-setting-plugins.registries"]')
      .scrollIntoViewIfNeeded();
    await page.screenshot({
      path: `output/playwright/plugin-registry-settings-${width}.png`,
    });

    // Navigation and search preserve the draft and stay in the selected scope.
    await sections
      .getByRole("button", { name: "Defaults", exact: true })
      .click();
    await expect(
      page.getByRole("checkbox", { name: "Agent Plugins", exact: true }),
    ).toHaveCount(0);
    await search.fill("OCI registries");
    await page.getByRole("button", { name: /OCI registries/ }).click();
    await expect(
      sidebar.getByRole("button", { name: "Global", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(
      page.locator('[id="managed-setting-plugins.registries"]'),
    ).toBeFocused();
    await expect(page.getByLabel("Exact registry origin")).toHaveValue(
      "https://registry.example",
    );
    await globalSave.click();
    await expect(globalSave).toBeDisabled();
    await page
      .getByRole("textbox", { name: "Included plugins", exact: true })
      .fill("temporary");
    await discard.click();
    await expect(
      page.getByRole("textbox", { name: "Included plugins", exact: true }),
    ).toHaveValue("colossus");

    await sidebar
      .getByRole("button", { name: "Workspace", exact: true })
      .click();
    await plugins.click();
    await expect(
      page.getByRole("textbox", { name: "Included plugins", exact: true }),
    ).toHaveValue("");
    await expect(
      page.getByRole("button", { name: /Review and apply r/ }),
    ).toBeVisible();
    const excluded = page.getByRole("textbox", {
      name: "Excluded plugins",
      exact: true,
    });
    await excluded.fill("example");
    await workspaceSave.click();
    await expect(workspaceSave).toBeDisabled();

    const workspace = sidebar.getByRole("combobox", {
      name: "Workspace",
      exact: true,
    });
    await workspace.click();
    await page
      .getByRole("option", { name: "Research Lab", exact: true })
      .click();
    await expect(excluded).toHaveValue("");
    await workspace.click();
    await page.getByRole("option", { name: "Colossus", exact: true }).click();
    await expect(excluded).toHaveValue("example");
    await page
      .locator('[id="managed-setting-plugins.exclude"]')
      .getByRole("button", { name: "Inherit" })
      .click();
    await expect(excluded).toHaveValue("");
    await discard.click();
    await expect(excluded).toHaveValue("example");

    await sections
      .getByRole("button", { name: "Advanced", exact: true })
      .click();
    await expect(page.getByText("Plugins", { exact: true })).toHaveCount(1);
    await search.fill("Excluded plugins");
    await page.getByRole("button", { name: /Excluded plugins/ }).click();
    await expect(
      sidebar.getByRole("button", { name: "Workspace", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(
      page.locator('[id="managed-setting-plugins.exclude"]'),
    ).toBeFocused();
    await expect(excluded).toHaveValue("example");
  });
}

test("plugin save uses the native defaults command and retains edits on failure", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Plugins", exact: true })
    .click();
  await page
    .getByRole("checkbox", { name: "Agent Plugins", exact: true })
    .uncheck();
  await page
    .getByRole("textbox", { name: "Excluded plugins", exact: true })
    .fill("example");
  await page.evaluate(() => {
    const state = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      settingsCalls: unknown[];
    };
    state.settingsCalls = [];
    state.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: unknown) => {
        state.settingsCalls.push({ command, args });
        throw {
          code: "revision_conflict",
          message: "The configuration changed. Reload before saving.",
          retryable: true,
          outcomeUnknown: false,
          violations: [],
        };
      },
    };
  });
  await page.getByRole("button", { name: "Save global changes" }).click();
  await expect(
    page.locator(".managed-settings-message[role=alert]"),
  ).toContainText("The configuration changed.");
  await expect(
    page.getByRole("checkbox", { name: "Agent Plugins", exact: true }),
  ).not.toBeChecked();
  await expect(
    page.getByRole("textbox", { name: "Excluded plugins", exact: true }),
  ).toHaveValue("example");
  const calls = await page.evaluate(
    () => (window as unknown as { settingsCalls: unknown[] }).settingsCalls,
  );
  expect(calls).toMatchObject([
    {
      command: "save_global_defaults",
      args: {
        request: {
          expectedRevision: 4,
          fieldOverrides: expect.arrayContaining([
            { fieldId: "plugins.enabled", value: false },
            { fieldId: "plugins.exclude", value: ["example"] },
          ]),
        },
      },
    },
  ]);
});
