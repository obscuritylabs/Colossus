import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const FIXTURE = "/?fixture=operations-studio";

test.beforeEach(async ({ page }) => {
  await page.goto(FIXTURE);
  await expect(
    page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
  ).toBeVisible();
});

test("minimum layout and capability-driven controls remain accessible", async ({
  page,
}) => {
  const layout = await page.evaluate(() => ({
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(layout).toEqual({
    innerWidth: 880,
    innerHeight: 640,
    scrollWidth: 880,
  });

  await expect(
    page.getByRole("button", { name: "Open files panel" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Open artifacts panel, 3 artifacts/u }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Attach a file" })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("button", { name: "Choose workspace context" }),
  ).toHaveCount(0);

  const results = await new AxeBuilder({ page }).analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);
});

test("slash commands complete and change Desktop modes without becoming prompts", async ({
  page,
}) => {
  const prompt = page.getByRole("textbox", { name: "Prompt" });

  await prompt.fill("/plan ");
  const commands = page.getByRole("listbox", { name: "Slash commands" });
  await expect(commands).toBeVisible();
  await expect(commands.getByRole("option")).toHaveCount(6);
  await expect(commands.getByRole("option").first()).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(prompt).toHaveAttribute(
    "aria-activedescendant",
    "desktop-slash-command-0",
  );
  await expect(
    commands.getByRole("option", { name: /\/plan new/u }),
  ).toBeVisible();

  await prompt.press("ArrowDown");
  await expect(commands.getByRole("option").nth(1)).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await prompt.press("Escape");
  await expect(commands).toBeHidden();

  await prompt.fill("/plan");
  await prompt.press("Enter");
  await expect(page.getByRole("radio", { name: "Plan" })).toBeChecked();
  await expect(prompt).toHaveValue("");
  await expect(page.getByText("Plan mode enabled.")).toBeVisible();

  await prompt.fill("/plan off");
  await prompt.press("Enter");
  await expect(page.getByRole("radio", { name: "Execute" })).toBeChecked();
  await expect(prompt).toHaveValue("");

  await prompt.fill("/plan execute direct");
  await prompt.press("Enter");
  await expect(
    page.getByText(/That Plan command is not available in Desktop/u),
  ).toBeVisible();
  await expect(prompt).toHaveValue("/plan execute direct");

  await prompt.fill("/research on");
  await prompt.press("Enter");
  await expect(page.getByRole("radio", { name: "Research" })).toBeChecked();
  await expect(page.getByText("Research mode enabled.")).toBeVisible();

  await prompt.fill("/execute");
  await prompt.press("Enter");
  await expect(page.getByRole("radio", { name: "Execute" })).toBeChecked();

  await prompt.fill("/permissions");
  await prompt.press("Enter");
  await expect(page.getByText("Desktop permission mode is ask.")).toBeVisible();

  await prompt.fill("/help");
  await prompt.press("Enter");
  await expect(prompt).toHaveValue("/");
  await expect(commands.getByRole("option")).toHaveCount(29);

  await prompt.fill("/plan list");
  await prompt.press("Enter");
  await expect(page.getByRole("button", { name: "Plans" })).toHaveAttribute(
    "aria-current",
    "page",
  );

  await prompt.fill("/work");
  await prompt.press("Enter");
  await expect(
    page.getByRole("button", { name: "Conversation" }),
  ).toHaveAttribute("aria-current", "page");
});

test("slash-command palette remains contained, scrollable, and accessible at compact width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 520, height: 720 });
  const prompt = page.getByRole("textbox", { name: "Prompt" });
  await prompt.fill("/");

  const menu = page.getByRole("listbox", { name: "Slash commands" });
  await expect(menu).toBeVisible();
  const palette = page.locator(".slash-command-menu");
  const geometry = await palette.evaluate((element) => {
    const menuRect = element.getBoundingClientRect();
    const tabsRect = document
      .querySelector(".session-workspace-tabs")
      ?.getBoundingClientRect();
    const composerRect = document
      .querySelector(".work-composer")
      ?.getBoundingClientRect();
    return {
      menuTop: menuRect.top,
      menuBottom: menuRect.bottom,
      menuHeight: menuRect.height,
      tabsBottom: tabsRect?.bottom ?? 0,
      composerTop: composerRect?.top ?? window.innerHeight,
      viewportHeight: window.innerHeight,
    };
  });
  expect(geometry.menuTop).toBeGreaterThanOrEqual(geometry.tabsBottom - 1);
  expect(geometry.menuBottom).toBeLessThanOrEqual(geometry.composerTop);
  expect(geometry.menuBottom).toBeLessThanOrEqual(geometry.viewportHeight);
  expect(geometry.menuHeight).toBeLessThanOrEqual(280);

  for (let index = 0; index < 18; index += 1) {
    await prompt.press("ArrowDown");
  }
  const activeDescendant = await prompt.getAttribute("aria-activedescendant");
  expect(activeDescendant).not.toBeNull();
  const activeRow = page.locator(`#${activeDescendant ?? "missing"}`);
  await expect(activeRow).toHaveAttribute("aria-selected", "true");
  const activeRowIsVisible = await activeRow.evaluate((element) => {
    const rowRect = element.getBoundingClientRect();
    const optionsRect = element.parentElement?.getBoundingClientRect();
    return (
      optionsRect !== undefined &&
      rowRect.top >= optionsRect.top &&
      rowRect.bottom <= optionsRect.bottom
    );
  });
  expect(activeRowIsVisible).toBe(true);

  const results = await new AxeBuilder({ page })
    .include(".slash-command-menu")
    .analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);

  await page
    .getByRole("heading", { name: "Harden desktop agent bootstrap" })
    .click();
  await expect(menu).toBeHidden();
});

test("settings dropdowns use styled app-owned menus with keyboard support", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();

  await expect(page.locator("select")).toHaveCount(0);
  const accessProfile = page.getByRole("combobox", {
    name: "Access profile",
    exact: true,
  });
  await accessProfile.click();

  const listbox = page.getByRole("listbox");
  await expect(listbox).toBeVisible();
  await expect(listbox.getByRole("option")).toHaveCount(5);
  await expect(
    listbox.getByRole("option", { name: "Allow all", exact: true }),
  ).toHaveAttribute("aria-selected", "true");

  await accessProfile.press("ArrowUp");
  await accessProfile.press("Enter");
  await expect(accessProfile).toContainText("Development");
  await accessProfile.click();
  await listbox.getByRole("option", { name: "Allow all", exact: true }).click();
  await expect(accessProfile).toContainText("Allow all");

  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Telemetry", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit Local collector", exact: true })
    .click();
  const protocol = page.getByRole("combobox", {
    name: "Protocol",
    exact: true,
  });
  await protocol.click();
  await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(2);

  const results = await new AxeBuilder({ page })
    .include(".app-select-popover")
    .analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);

  await protocol.press("Escape");
  await expect(page.getByRole("listbox")).toHaveCount(0);
});

test("appearance preferences are readable, consistent, and persistent", async ({
  page,
}) => {
  test.setTimeout(60_000);
  const globalTabs = [
    "Providers",
    "Models",
    "Credentials",
    "MCP",
    "Search",
    "Telemetry",
    "Defaults",
    "Desktop",
  ] as const;
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Desktop", exact: true }).click();

  const colorTheme = page.getByRole("combobox", {
    name: /Color theme/u,
  });
  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Light", exact: true })
    .click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme-preference",
    "light",
  );
  await page.waitForTimeout(160);

  for (const tab of globalTabs) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    const lightAccessibility = await new AxeBuilder({ page })
      .include(".managed-settings-body")
      .analyze();
    expect(
      lightAccessibility.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
      `${tab} should remain accessible in the light theme`,
    ).toEqual([]);
  }

  for (const [tab, rowSelector] of [
    ["Providers", ".provider-row"],
    ["Models", ".model-row"],
    ["Credentials", ".credential-list .managed-list-row"],
    ["Search", ".search-profile-row"],
    ["Telemetry", ".telemetry-row"],
  ] as const) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    const row = page.locator(rowSelector).first();
    await row.hover();
    await expect(row).toHaveCSS("background-color", "rgb(230, 237, 246)");
    const hoverAccessibility = await new AxeBuilder({ page })
      .include(rowSelector)
      .analyze();
    expect(
      hoverAccessibility.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
      `${tab} hover state should remain accessible in the light theme`,
    ).toEqual([]);
  }

  await page.getByRole("button", { name: "Desktop", exact: true }).click();
  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Dark", exact: true })
    .click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.waitForTimeout(160);

  for (const tab of globalTabs) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    const darkAccessibility = await new AxeBuilder({ page })
      .include(".managed-settings-body")
      .analyze();
    expect(
      darkAccessibility.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
      `${tab} should remain accessible in the dark theme`,
    ).toEqual([]);
  }

  const textSize = page.getByRole("combobox", { name: /Text size/u });
  for (const [label, value, expectedRootSize] of [
    ["Compact", "compact", "15px"],
    ["Comfortable", "comfortable", "16px"],
    ["Large", "large", "18px"],
  ] as const) {
    await textSize.click();
    await page
      .getByRole("listbox")
      .getByRole("option", { name: label, exact: true })
      .click();
    await expect(page.locator("html")).toHaveAttribute("data-text-size", value);
    await expect
      .poll(() =>
        page
          .locator("html")
          .evaluate((element) => getComputedStyle(element).fontSize),
      )
      .toBe(expectedRootSize);
  }

  const paneWidths: number[] = [];
  for (const tab of globalTabs) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    const dimensions = await page
      .locator(".managed-settings-body")
      .evaluate((element) => ({
        width: element.getBoundingClientRect().width,
        shellClientWidth:
          element.closest(".managed-settings-shell")?.clientWidth ?? 0,
        shellScrollWidth:
          element.closest(".managed-settings-shell")?.scrollWidth ?? 0,
      }));
    paneWidths.push(dimensions.width);
    expect(dimensions.width).toBeLessThanOrEqual(1180);
    expect(dimensions.shellScrollWidth).toBeLessThanOrEqual(
      dimensions.shellClientWidth + 1,
    );
  }
  expect(Math.max(...paneWidths) - Math.min(...paneWidths)).toBeLessThanOrEqual(
    1,
  );

  await page.getByRole("button", { name: "Providers", exact: true }).click();
  const readableCopySizes = await page
    .locator(
      ".managed-heading-copy, .managed-settings-body small, .managed-settings-body .eyebrow",
    )
    .evaluateAll((elements) =>
      elements
        .filter((element) => {
          const style = getComputedStyle(element);
          return style.display !== "none" && style.visibility !== "hidden";
        })
        .map((element) =>
          Number.parseFloat(getComputedStyle(element).fontSize),
        ),
    );
  expect(Math.min(...readableCopySizes)).toBeGreaterThanOrEqual(13.5);

  await page.setViewportSize({ width: 700, height: 800 });
  for (const tab of globalTabs) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    const bounds = await page
      .locator(".managed-settings-shell")
      .evaluate((element) => ({
        clientWidth: element.clientWidth,
        scrollWidth: element.scrollWidth,
      }));
    expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
  }

  const compactNavigationBounds = await page
    .getByRole("button", { name: "Open Workspace navigation", exact: true })
    .evaluate((element) => element.getBoundingClientRect().toJSON());
  const compactHeadingBounds = await page
    .getByRole("heading", { name: "Desktop settings", exact: true })
    .evaluate((element) => element.getBoundingClientRect().toJSON());
  expect(compactNavigationBounds.right).toBeLessThanOrEqual(
    compactHeadingBounds.left,
  );

  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.locator("html")).toHaveAttribute("data-text-size", "large");
});

test("system theme follows operating-system color changes", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme-preference",
    "system",
  );

  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("light theme keeps session inspection surfaces readable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Desktop", exact: true }).click();

  const colorTheme = page.getByRole("combobox", { name: /Color theme/u });
  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Light", exact: true })
    .click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "Work", exact: true }).click();

  await page.getByRole("button", { name: "Topology", exact: true }).click();
  await expect(page.locator(".session-map-stage")).toHaveCSS(
    "background-color",
    "rgb(255, 255, 255)",
  );
  await expect(page.locator(".session-map-family").first()).toHaveCSS(
    "background-color",
    "rgb(247, 249, 252)",
  );

  await page.getByRole("button", { name: "Snapshots", exact: true }).click();
  await expect(
    page.locator(".session-snapshot-list article").first(),
  ).toHaveCSS("background-color", "rgb(247, 249, 252)");

  await page.getByRole("button", { name: "Resources", exact: true }).click();
  await expect(
    page.locator(".session-resource-groups > button").first(),
  ).toHaveCSS("background-color", "rgb(247, 249, 252)");

  const detailsTrigger = page.getByRole("button", {
    name: /thread details$/u,
  });
  if ((await detailsTrigger.getAttribute("aria-expanded")) !== "true") {
    await detailsTrigger.click();
  }
  const details = page.getByRole("complementary", {
    name: "Thread details",
  });
  await expect(details).toHaveCSS("background-color", "rgb(240, 244, 248)");
  await expect(details.locator(".thread-details-list > div").first()).toHaveCSS(
    "background-color",
    "rgb(247, 249, 252)",
  );

  const results = await new AxeBuilder({ page })
    .include(".thread-details-panel")
    .analyze();
  expect(
    results.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);

  await details.getByRole("button", { name: /All resources/u }).click();
  await expect(details).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Resources" })).toBeVisible();
});

test("light theme keeps workspace shell surfaces readable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Desktop", exact: true }).click();

  const colorTheme = page.getByRole("combobox", { name: /Color theme/u });
  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Light", exact: true })
    .click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

  await page.getByRole("button", { name: "Work", exact: true }).click();
  await expect(
    page
      .getByRole("navigation", { name: "Workspace destinations" })
      .getByRole("button", { name: "Activity", exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .getByRole("navigation", { name: "Session views" })
      .getByRole("button", { name: "Activity", exact: true }),
  ).toBeVisible();
  await expect(
    page.locator(".connection-badge.connection-connected"),
  ).toHaveCSS("background-color", "rgb(230, 245, 238)");
  await expect(page.locator(".markdown-content table")).toHaveCSS(
    "background-color",
    "rgb(255, 255, 255)",
  );
  await expect(page.locator(".markdown-content th").first()).toHaveCSS(
    "background-color",
    "rgb(237, 243, 249)",
  );
  await expect(page.locator(".work-composer-dock")).toHaveCSS(
    "background-color",
    "rgb(240, 244, 248)",
  );
  await expect(page.locator(".work-composer")).toHaveCSS(
    "background-color",
    "rgb(255, 255, 255)",
  );
  await expect(
    page.locator('.space-destinations button[aria-current="page"]'),
  ).toHaveCSS("background-color", "rgb(229, 239, 255)");

  const workAccessibility = await new AxeBuilder({ page })
    .include(".work-surface")
    .analyze();
  expect(
    workAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);

  await page
    .locator(".space-destinations")
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await expect(page.locator(".space-settings-context")).toHaveCSS(
    "background-color",
    "rgb(237, 243, 249)",
  );
  await expect(page.locator(".authority-summary")).toHaveCSS(
    "background-color",
    "rgb(240, 244, 248)",
  );
  await expect(page.locator(".authority-item").first()).toHaveCSS(
    "background-color",
    "rgb(255, 255, 255)",
  );
  await expect(page.locator(".managed-settings-actions")).toHaveCSS(
    "background-color",
    "rgb(247, 249, 252)",
  );

  const settingsAccessibility = await new AxeBuilder({ page })
    .include(".managed-settings-shell")
    .analyze();
  expect(
    settingsAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
});

test("artifact, file, and workspace destination surfaces follow both themes", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Desktop", exact: true }).click();

  const colorTheme = page.getByRole("combobox", { name: /Color theme/u });
  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Light", exact: true })
    .click();
  await page.getByRole("button", { name: "Work", exact: true }).click();

  await page
    .getByRole("button", { name: /Open artifacts panel, 3 artifacts/u })
    .click();
  const artifactPanel = page.getByRole("complementary", {
    name: "Work artifacts",
  });
  await expect(artifactPanel).toHaveCSS(
    "background-color",
    "rgb(247, 249, 252)",
  );
  await expect(artifactPanel.locator(".artifact-tabs")).toHaveCSS(
    "background-color",
    "rgb(237, 243, 249)",
  );
  await expect(artifactPanel.locator(".artifact-preview")).toHaveCSS(
    "background-color",
    "rgb(247, 249, 252)",
  );
  await expect(artifactPanel.locator(".artifact-preview pre")).toHaveCSS(
    "font-size",
    "13px",
  );
  const lightArtifactAccessibility = await new AxeBuilder({ page })
    .include(".artifact-workspace")
    .analyze();
  expect(
    lightArtifactAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close artifacts drawer" }).click();

  await page.getByRole("button", { name: "Open files panel" }).click();
  const filePanel = page.locator(".workspace-files-drawer");
  await expect(filePanel.locator(".file-explorer")).toHaveCSS(
    "background-color",
    "rgb(237, 242, 247)",
  );
  await expect(filePanel.locator(".file-code-scroll")).toHaveCSS(
    "background-color",
    "rgb(247, 249, 252)",
  );
  const lightFileAccessibility = await new AxeBuilder({ page })
    .include(".workspace-files-drawer")
    .analyze();
  expect(
    lightFileAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close files drawer" }).click();

  for (const [destination, selector, expectedBackground] of [
    ["Capabilities", ".overview-section", "rgb(255, 255, 255)"],
    ["Library", ".artifact-library-list article", "rgb(255, 255, 255)"],
    ["Connections", ".target-grid > button:nth-child(2)", "rgb(240, 244, 248)"],
  ] as const) {
    await page
      .locator(".space-destinations")
      .getByRole("button", { name: destination, exact: true })
      .click();
    await expect(page.locator(selector).first()).toHaveCSS(
      "background-color",
      expectedBackground,
    );
    const destinationAccessibility = await new AxeBuilder({ page })
      .include(".operations-surface")
      .analyze();
    expect(
      destinationAccessibility.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
      `${destination} should remain accessible in the light theme`,
    ).toEqual([]);
  }

  await page
    .locator(".space-destinations")
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Desktop", exact: true }).click();
  await colorTheme.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Dark", exact: true })
    .click();
  await page.getByRole("button", { name: "Work", exact: true }).click();
  await page
    .getByRole("button", { name: /Open artifacts panel, 3 artifacts/u })
    .click();
  await expect(page.locator(".artifact-preview")).toHaveCSS(
    "background-color",
    "rgb(8, 18, 30)",
  );
  await expect(page.locator(".artifact-tabs")).toHaveCSS(
    "background-color",
    "rgb(17, 34, 55)",
  );
});

test("managed settings expose complete catalog editors without horizontal overflow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();

  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByRole("button", { name: "Edit primary", exact: true }).click();
  const modelLabel = page.getByRole("textbox", {
    name: /^Display label/u,
  });
  const reasoningEffort = page.getByRole("combobox", {
    name: "Reasoning effort",
  });
  await expect(reasoningEffort).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Provider connection", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Token limits", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Supported features", exact: true }),
  ).toBeVisible();
  const modelLabelHeight = await modelLabel.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  const reasoningFieldHeight = await reasoningEffort.evaluate(
    (element) => element.parentElement?.getBoundingClientRect().height ?? 0,
  );
  expect(Math.abs(modelLabelHeight - reasoningFieldHeight)).toBeLessThanOrEqual(
    1,
  );
  for (const label of ["Tool calls", "Streaming", "Image inputs"]) {
    const capability = page.getByRole("switch", { name: new RegExp(label) });
    await expect(capability).toBeVisible();
    const dimensions = await capability.evaluate((element) => ({
      height: element.getBoundingClientRect().height,
      width: element.getBoundingClientRect().width,
    }));
    expect(dimensions.width).toBeLessThanOrEqual(36);
    expect(dimensions.height).toBeLessThanOrEqual(20);
  }
  const modelResults = await new AxeBuilder({ page })
    .include(".models-settings")
    .analyze();
  expect(
    modelResults.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close model editor" }).click();

  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Connection", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Authentication", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Runtime", exact: true }),
  ).toBeVisible();
  await expect(page.getByPlaceholder("Provider default")).toBeVisible();
  const providerAdapter = page.getByRole("combobox", { name: /^Adapter/u });
  await providerAdapter.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Codex subscription", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: /^Endpoint URL/u }),
  ).toBeDisabled();
  await expect(
    page.getByRole("combobox", { name: /^Credential reference/u }),
  ).toBeDisabled();
  const providerResults = await new AxeBuilder({ page })
    .include(".providers-settings")
    .analyze();
  expect(
    providerResults.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close provider editor" }).click();

  await page.getByRole("button", { name: "Credentials", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Credentials", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Stored securely on this device", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Values are entered in a system dialog and are not shown again.",
      {
        exact: true,
      },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Add credential", exact: true }),
  ).toBeVisible();
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
  const githubCredential = page
    .getByRole("listitem")
    .filter({ hasText: "GitHub workspace token" });
  const documentationCredential = page
    .getByRole("listitem")
    .filter({ hasText: "Documentation API key" });
  await expect(
    githubCredential.getByText("Used by 2", { exact: true }),
  ).toBeVisible();
  await expect(
    documentationCredential.getByText("Used by 3", { exact: true }),
  ).toBeVisible();
  const credentialsAccessibility = await new AxeBuilder({ page })
    .include(".credentials-settings")
    .analyze();
  expect(
    credentialsAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);

  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Search services", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Available by workspace", { exact: true }),
  ).toBeVisible();
  const engineeringSearch = page
    .getByRole("listitem")
    .filter({ hasText: "Engineering search" });
  await expect(
    engineeringSearch.getByText("Credential attached", { exact: true }),
  ).toBeVisible();
  await expect(
    engineeringSearch.getByText("Used by 1", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Edit Engineering search", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Identity", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Connection", exact: true }),
  ).toBeVisible();
  const searchLabel = page.getByLabel("Display label", { exact: false });
  const searchAdapter = page.getByRole("combobox", {
    name: /Adapter/u,
  });
  const searchLabelHeight = await searchLabel.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  const searchAdapterHeight = await searchAdapter.evaluate(
    (element) => element.parentElement?.getBoundingClientRect().height ?? 0,
  );
  expect(Math.abs(searchLabelHeight - searchAdapterHeight)).toBeLessThanOrEqual(
    1,
  );
  await expect(
    page.getByLabel("Authentication header", { exact: false }),
  ).not.toBeVisible();
  await page.getByText("Request controls", { exact: true }).click();
  await expect(
    page.getByLabel("Authentication header", { exact: false }),
  ).toBeVisible();
  await expect(page.getByLabel("Timeout (ms)", { exact: false })).toBeVisible();

  const searchAccessibility = await new AxeBuilder({ page })
    .include(".search-settings")
    .analyze();
  expect(
    searchAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close search editor" }).click();

  await page.getByRole("button", { name: "Telemetry", exact: true }).click();
  await expect(
    page.getByText("Review shared data", { exact: true }),
  ).toBeVisible();
  const localCollector = page
    .getByRole("listitem")
    .filter({ hasText: "Local collector" });
  await expect(
    localCollector.getByText("OTLP gRPC", { exact: true }),
  ).toBeVisible();
  await expect(
    localCollector.getByText("3 OTLP signals", { exact: true }),
  ).toBeVisible();
  await expect(
    localCollector.getByText("Metadata only", { exact: true }),
  ).toBeVisible();
  await expect(
    localCollector.getByText("Used by 1", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Edit Local collector", exact: true })
    .click();
  for (const heading of [
    "Identity",
    "Collector destination",
    "Signals",
    "Audit record content",
    "Resource attributes",
  ]) {
    await expect(
      page.getByRole("heading", { name: heading, exact: true }),
    ).toBeVisible();
  }
  const telemetryLabel = page.getByLabel("Display label", { exact: false });
  const telemetryProtocol = page.getByRole("combobox", {
    name: "Protocol",
    exact: true,
  });
  const telemetryLabelHeight = await telemetryLabel.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  const telemetryProtocolHeight = await telemetryProtocol.evaluate(
    (element) => element.parentElement?.getBoundingClientRect().height ?? 0,
  );
  expect(
    Math.abs(telemetryLabelHeight - telemetryProtocolHeight),
  ).toBeLessThanOrEqual(1);
  await page.getByRole("switch", { name: "Traces", exact: true }).uncheck();
  await expect(
    page.getByRole("spinbutton", { name: "Trace sample (millionths)" }),
  ).toBeDisabled();
  await page.getByRole("switch", { name: "Traces", exact: true }).check();
  const journalPayloads = page.getByRole("combobox", {
    name: "Audit content",
    exact: true,
  });
  await journalPayloads.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Full sensitive payloads", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("switch", {
      name: "Acknowledge sensitive journal content",
      exact: true,
    })
    .check();
  await expect(
    page.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeEnabled();
  const telemetryAccessibility = await new AxeBuilder({ page })
    .include(".telemetry-settings")
    .analyze();
  expect(
    telemetryAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close telemetry editor" }).click();

  await page.getByRole("button", { name: "MCP", exact: true }).click();
  const splunkRow = page.getByRole("row", {
    name: /splunk-search streamable http/u,
  });
  await expect(
    splunkRow.getByText("splunk-search", { exact: true }),
  ).toHaveCount(1);
  await page
    .getByRole("button", { name: "Edit splunk-search", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Connection", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Access", exact: true }),
  ).toBeVisible();
  const serverName = page.getByLabel("Server name", { exact: false });
  const transport = page.getByRole("combobox", {
    name: "Transport",
    exact: true,
  });
  await expect(serverName).toBeVisible();
  await expect(transport).toBeVisible();
  const serverNameHeight = await serverName.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  const transportFieldHeight = await transport.evaluate(
    (element) => element.parentElement?.getBoundingClientRect().height ?? 0,
  );
  expect(Math.abs(serverNameHeight - transportFieldHeight)).toBeLessThanOrEqual(
    1,
  );
  await expect(page.getByLabel("Research tool projections")).not.toBeVisible();
  await page.getByText("Advanced settings", { exact: true }).click();
  for (const label of [
    "Static headers",
    "Research tool projections",
    "Timeout (ms)",
    "Maximum output (bytes)",
  ]) {
    await expect(page.getByLabel(label, { exact: true })).toBeVisible();
  }
  await expect(
    page.getByText("Credential headers", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("OAuth client configuration", { exact: true }),
  ).toBeVisible();

  const editorWidth = await page
    .locator(".mcp-server-editor")
    .evaluate((element) => element.getBoundingClientRect().width);
  expect(editorWidth).toBeLessThanOrEqual(1080);

  const editorAccessibility = await new AxeBuilder({ page })
    .include(".mcp-server-editor")
    .analyze();
  expect(
    editorAccessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Close MCP editor" }).click();

  await page.getByRole("button", { name: "Defaults", exact: true }).click();
  await expect(
    page.getByText("Set the values workspaces use unless they override them.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByText("Stop a run after this many agent turns.", { exact: true }),
  ).toBeVisible();
  await page.getByText("Advanced defaults", { exact: true }).click();
  const repeatedDescriptions = await page
    .locator(".managed-field-row")
    .evaluateAll(
      (rows) =>
        rows.filter((row) => {
          const title = row.querySelector("strong")?.textContent?.trim();
          const description = row
            .querySelector("div > span")
            ?.textContent?.trim();
          return title === description;
        }).length,
    );
  expect(repeatedDescriptions).toBe(0);

  await page.getByRole("button", { name: "Desktop", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Desktop settings", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Manage the workspace and local services that belong to this Desktop installation.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByText("Desktop-only controls", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Managed workspace", { exact: true }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "This Desktop", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Saved runtimes", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Configure runtime", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add external runtime", exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("credential management stays secure, clear, and compact", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 800 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Credentials", exact: true }).click();

  await expect(
    page.getByText("Stored securely on this device", { exact: true }),
  ).toBeVisible();
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
  await page
    .getByLabel("Display label", { exact: false })
    .fill("Splunk admin token");
  const credentialType = page.getByRole("combobox", {
    name: /Credential type/u,
  });
  await credentialType.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Bearer token", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Add credential", exact: true })
    .click();

  const addedCredential = page
    .getByRole("listitem")
    .filter({ hasText: "Splunk admin token" });
  await expect(addedCredential).toBeVisible();
  await expect(
    addedCredential.getByText("Not referenced", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Credential stored securely.", { exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("search services stay complete, clear, and compact", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 800 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Search", exact: true }).click();

  await expect(
    page.getByText("Adding a service here does not enable it automatically.", {
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Add search service", exact: true })
    .click();
  await page.getByLabel("Display label", { exact: false }).fill("Web results");
  await page.getByLabel("Profile ID", { exact: false }).fill("web-results");
  const adapter = page.getByRole("combobox", { name: /Adapter/u });
  await adapter.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "SerpAPI", exact: true })
    .click();
  await expect(
    page.getByText("Credential required", { exact: true }),
  ).toBeVisible();
  await page
    .getByLabel("Endpoint URL", { exact: false })
    .fill("https://serpapi.com/search");

  const credential = page.getByRole("combobox", {
    name: /Credential reference/u,
  });
  await page
    .locator(".search-editor")
    .getByRole("button", { name: "Add search service" })
    .click();
  await expect(credential).toBeFocused();
  await expect(credential).toHaveAttribute("aria-invalid", "true");

  await credential.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Documentation API key", exact: true })
    .click();
  await page.getByText("Request controls", { exact: true }).click();
  await expect(
    page.getByLabel("Authentication header", { exact: false }),
  ).toHaveCount(0);
  await page.getByLabel("Timeout (ms)", { exact: false }).fill("45000");
  await page
    .locator(".search-editor")
    .getByRole("button", { name: "Add search service" })
    .click();

  const addedProfile = page
    .getByRole("listitem")
    .filter({ hasText: "Web results" });
  await expect(addedProfile).toBeVisible();
  await expect(
    addedProfile.getByText("Credential attached", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("Not used", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("45s · v1", { exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("every settings tab fills the viewport and keeps actions anchored", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1100 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const scrollPane = page.locator(".settings-scroll");
  const sectionTabs = page.getByRole("navigation", {
    name: "Settings sections",
  });
  const measureGaps = () =>
    scrollPane.evaluate((element) => {
      const operationsBounds = element
        .closest(".operations-surface")
        ?.getBoundingClientRect();
      const actionBounds = element
        .querySelector(".managed-settings-actions")
        ?.getBoundingClientRect();
      return {
        actionGap:
          operationsBounds && actionBounds
            ? operationsBounds.bottom - actionBounds.bottom
            : null,
        viewportGap: operationsBounds
          ? operationsBounds.bottom - element.getBoundingClientRect().bottom
          : null,
      };
    });

  const workspaceTabs = [
    "Runtime",
    "Providers",
    "MCP",
    "Access",
    "Sandbox",
    "Search",
    "Telemetry",
    "Research",
    "Advanced",
    "Effective YAML",
  ];
  for (const tab of workspaceTabs) {
    await sectionTabs.getByRole("button", { name: tab, exact: true }).click();
    const gaps = await measureGaps();
    expect(
      Math.abs(gaps.viewportGap ?? -1),
      `${tab} viewport gap`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(gaps.actionGap ?? -1),
      `${tab} action gap`,
    ).toBeLessThanOrEqual(1);
  }

  await page.getByRole("button", { name: "Global", exact: true }).click();
  for (const tab of [
    "Providers",
    "Models",
    "Credentials",
    "MCP",
    "Search",
    "Telemetry",
    "Defaults",
    "Desktop",
  ]) {
    await sectionTabs.getByRole("button", { name: tab, exact: true }).click();
    const gaps = await measureGaps();
    expect(
      Math.abs(gaps.viewportGap ?? -1),
      `${tab} viewport gap`,
    ).toBeLessThanOrEqual(1);
    if (tab === "Defaults") {
      expect(
        Math.abs(gaps.actionGap ?? -1),
        "Defaults action gap",
      ).toBeLessThanOrEqual(1);
    } else {
      expect(gaps.actionGap).toBeNull();
    }
  }

  await page.setViewportSize({ width: 700, height: 640 });
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  for (const tab of workspaceTabs) {
    await sectionTabs.getByRole("button", { name: tab, exact: true }).click();
    const gaps = await measureGaps();
    expect(
      Math.abs(gaps.viewportGap ?? -1),
      `${tab} compact viewport gap`,
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(gaps.actionGap ?? -1),
      `${tab} compact action gap`,
    ).toBeLessThanOrEqual(1);
  }

  await sectionTabs.getByRole("button", { name: "MCP", exact: true }).click();
  const scrollMetrics = await scrollPane.evaluate((element) => {
    element.scrollTop = Math.floor(element.scrollHeight / 2);
    return {
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
      scrollTop: element.scrollTop,
    };
  });
  expect(scrollMetrics.scrollHeight).toBeGreaterThan(
    scrollMetrics.clientHeight,
  );
  expect(scrollMetrics.scrollTop).toBeGreaterThan(0);
  const stickyGaps = await measureGaps();
  expect(Math.abs(stickyGaps.actionGap ?? -1)).toBeLessThanOrEqual(1);
});

test("settings success feedback uses a dismissible toast", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Defaults", exact: true }).click();
  await page.getByRole("combobox", { name: "Access profile" }).click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Pinned", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Save global changes", exact: true })
    .click();

  const toastRegion = page.locator(".toast-region");
  await expect(toastRegion.getByText("Global changes saved.")).toBeVisible();
  await expect(
    page.locator(".managed-settings-message.is-success"),
  ).toHaveCount(0);
  await toastRegion
    .getByRole("button", { name: "Dismiss notification" })
    .click();
  await expect(toastRegion.getByText("Global changes saved.")).toHaveCount(0);
});

test("model settings stay clear, complete, and compact", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 800 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();

  await expect(
    page.getByText(
      "Turn on only the features this model supports: tools, streaming, and images.",
      {
        exact: true,
      },
    ),
  ).toBeVisible();
  const primaryModel = page
    .getByRole("listitem")
    .filter({ hasText: "primary" });
  await expect(primaryModel.getByText("Tools", { exact: true })).toBeVisible();
  await expect(
    primaryModel.getByText("Streaming", { exact: true }),
  ).toBeVisible();
  await expect(
    primaryModel.getByText("Used by 4", { exact: true }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Add model", exact: true }).click();
  await page
    .getByRole("textbox", { name: /^Display label/u })
    .fill("Vision model");
  await page.getByRole("textbox", { name: /^Profile ID/u }).fill("vision");
  await page
    .getByRole("textbox", { name: /^Model identifier/u })
    .fill("fixture-vision");
  await page
    .getByRole("spinbutton", { name: /^Context window \(tokens\)/u })
    .fill("256000");
  await page
    .getByRole("spinbutton", { name: /^Maximum output \(tokens\)/u })
    .fill("32000");
  await page.getByRole("switch", { name: /Image inputs/u }).check();
  const reasoningEffort = page.getByRole("combobox", {
    name: /^Reasoning effort/u,
  });
  await reasoningEffort.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "high", exact: true })
    .click();
  await page
    .locator(".model-editor")
    .getByRole("button", { name: "Add model" })
    .click();

  const addedModel = page
    .getByRole("listitem")
    .filter({ hasText: "Vision model" });
  await expect(addedModel).toBeVisible();
  await expect(addedModel.getByText("Images", { exact: true })).toBeVisible();
  await expect(addedModel.getByText("Not used", { exact: true })).toBeVisible();
  await expect(
    addedModel.getByText("256k context · 32k output · v1", { exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("provider connections stay clear, secure, and compact", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 800 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();

  await expect(
    page.getByText(
      "Colossus stores which credential to use, not its secret value.",
      { exact: true },
    ),
  ).toBeVisible();
  const primaryProvider = page
    .getByRole("listitem")
    .filter({ hasText: "primary-provider" });
  await expect(
    primaryProvider.getByText("OpenAI compatible", { exact: true }),
  ).toBeVisible();
  await expect(
    primaryProvider.getByText("Used by 1", { exact: true }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  await page
    .getByRole("textbox", { name: /^Display label/u })
    .fill("OpenAI production");
  await page
    .getByRole("textbox", { name: /^Profile ID/u })
    .fill("openai-production");
  const adapter = page.getByRole("combobox", { name: /^Adapter/u });
  await adapter.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "OpenAI Responses", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: /^Endpoint URL/u })
    .fill("https://api.openai.com/v1");
  const credential = page.getByRole("combobox", {
    name: /^Credential reference/u,
  });
  await credential.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Documentation API key", exact: true })
    .click();
  await page
    .getByRole("spinbutton", { name: /^Request timeout \(ms\)/u })
    .fill("45000");
  await page
    .locator(".provider-editor")
    .getByRole("button", { name: "Add provider" })
    .click();

  const addedProvider = page
    .getByRole("listitem")
    .filter({ hasText: "OpenAI production" });
  await expect(addedProvider).toBeVisible();
  await expect(
    addedProvider.getByText("OpenAI Responses", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProvider.getByText("Credential attached", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProvider.getByText("No models", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProvider.getByText("45s timeout · v1", { exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("telemetry connections stay clear and compact", async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 800 });
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Telemetry", exact: true }).click();

  await expect(
    page.getByText(
      "Full audit records can include prompts, responses, and tool input or output.",
      { exact: true },
    ),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Add telemetry connection", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: /^Display label/u })
    .fill("Production observability");
  await page
    .getByRole("textbox", { name: /^Service name/u })
    .fill("colossus-production");
  await page
    .getByRole("textbox", { name: /^Collector endpoint/u })
    .fill("https://otel.example.test:4318");
  const protocol = page.getByRole("combobox", {
    name: "Protocol",
    exact: true,
  });
  await protocol.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "OTLP HTTP/protobuf", exact: true })
    .click();
  await page
    .getByRole("spinbutton", { name: "Timeout (ms)", exact: true })
    .fill("15000");
  await page.getByRole("switch", { name: "Metrics", exact: true }).uncheck();
  await page.getByRole("switch", { name: "JSON stdout", exact: true }).check();
  const journalPayloads = page.getByRole("combobox", {
    name: "Audit content",
    exact: true,
  });
  await journalPayloads.click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Disabled", exact: true })
    .click();
  await page
    .locator(".telemetry-editor")
    .getByRole("button", { name: "Add telemetry connection" })
    .click();

  const addedProfile = page
    .getByRole("listitem")
    .filter({ hasText: "Production observability" });
  await expect(addedProfile).toBeVisible();
  await expect(
    addedProfile.getByText("otel.example.test:4318", { exact: false }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("OTLP HTTP/protobuf", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("2 OTLP signals", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("Audit content off", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("Not used", { exact: true }),
  ).toBeVisible();
  await expect(
    addedProfile.getByText("15s · v1", { exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("session snapshots and every durable record family are listable and inspectable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Snapshots", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Context snapshots" }),
  ).toBeVisible();
  await expect(page.getByText(/Messages 1–18/u)).toBeVisible();
  const preview = page.locator(".session-snapshot-preview").first();
  await expect(
    preview.getByRole("heading", { name: "Session context" }),
  ).toBeVisible();
  const previewHeight = await preview.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(previewHeight.clientHeight).toBeLessThan(previewHeight.scrollHeight);
  await page.getByRole("button", { name: "View snapshot" }).first().click();
  const details = page.getByLabel("Thread details", { exact: true });
  await expect(
    details.getByText("Context snapshot", { exact: true }),
  ).toBeVisible();
  await expect(
    details.getByRole("heading", { name: "Messages 1–18" }),
  ).toBeVisible();
  await expect(
    details.getByRole("heading", { name: "Session context" }),
  ).toBeVisible();
  await expect(
    details.getByRole("heading", { name: "Pinned facts", exact: true }),
  ).toBeVisible();
  await expect(
    details.getByRole("heading", { name: "Open tasks", exact: true }),
  ).toBeVisible();
  const snapshotPanel = details.locator(".session-map-details");
  const scrollState = await snapshotPanel.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
    return {
      clientHeight: element.clientHeight,
      overflowY: getComputedStyle(element).overflowY,
      scrollHeight: element.scrollHeight,
      scrollTop: element.scrollTop,
    };
  });
  expect(scrollState.overflowY).toBe("auto");
  expect(scrollState.scrollHeight).toBeGreaterThan(scrollState.clientHeight);
  expect(scrollState.scrollTop).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Back to thread details" }).click();

  await page.getByRole("button", { name: "Resources", exact: true }).click();
  const resources = page.getByRole("region", { name: "Resources" });
  for (const label of [
    "Delegated agents",
    "Goals",
    "Tasks",
    "Key decisions",
    "Memories",
    "Research",
  ]) {
    await expect(resources.getByText(label, { exact: true })).toBeVisible();
  }
  await resources.getByText("Key decisions", { exact: true }).click();
  await resources
    .getByRole("button", { name: /Keep execution boundary unchanged/u })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Keep execution boundary unchanged",
      exact: true,
    }),
  ).toBeVisible();
});

test("required app-owned dropdowns retain form validation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByRole("button", { name: "Add model", exact: true }).click();

  const provider = page.getByRole("combobox", {
    name: /^Provider profile/u,
  });
  await provider.click();
  await page
    .getByRole("option", { name: "Select provider", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Display label" })
    .fill("Validation model");
  await page
    .getByRole("textbox", { name: "Profile ID" })
    .fill("validation-model");
  await page
    .getByRole("textbox", { name: "Model identifier" })
    .fill("fixture-validation");
  await page
    .locator(".model-editor")
    .getByRole("button", { name: "Add model" })
    .click();

  await expect(
    page.getByRole("heading", { name: "Add model", exact: true }),
  ).toBeVisible();
  await expect(provider).toBeFocused();
  await expect(provider).toHaveAttribute("aria-invalid", "true");
});

test("Workspace archive and restore update the sidebar and confirm the outcome", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  await page.getByRole("button", { name: "Manage Workspaces" }).click();
  await page.getByRole("button", { name: "Archive Colossus" }).click();

  await expect(page.getByRole("status")).toContainText("Archived Colossus.");
  await expect(
    page.locator(".space-shelf.is-active", { hasText: "Colossus" }),
  ).toHaveCount(0);
  await expect(page.locator(".space-shelf.is-active")).toContainText(
    "Research Lab",
  );

  await page.getByRole("button", { name: "Manage Workspaces" }).click();
  const restore = page.getByRole("button", { name: "Restore Colossus" });
  await expect(restore).toBeVisible();
  await restore.click();

  await expect(page.getByRole("status")).toContainText(
    "Restored Colossus. Select it when you’re ready.",
  );
  await expect(
    page.getByRole("button", { name: "Restore Colossus" }),
  ).toHaveCount(0);
});

test("Research settings expose depth and evidence choices and restore focus", async ({
  page,
}) => {
  const researchMode = page.getByRole("radio", { name: "Research" });
  await page
    .locator(".mode-switch")
    .getByText("Research", { exact: true })
    .click();
  await expect(researchMode).toBeChecked();

  const settingsTrigger = page.getByLabel(
    "Research controls, sources This Workspace",
  );
  await settingsTrigger.click();

  await expect(
    page.getByRole("heading", { name: "Research settings" }),
  ).toBeVisible();
  await expect(page.getByRole("radio", { name: "Standard" })).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: /This Workspace/u }),
  ).toBeChecked();

  await page.locator(".research-depth-option", { hasText: "Deep" }).click();
  await page.locator(".research-source-option", { hasText: "Web" }).click();
  await expect(page.getByRole("radio", { name: "Deep" })).toBeChecked();
  await expect(
    page.getByLabel("Research controls, sources This Workspace, Web"),
  ).toBeVisible();

  await page.getByRole("button", { name: "Close research settings" }).click();
  const updatedTrigger = page.getByLabel(
    "Research controls, sources This Workspace, Web",
  );
  await expect(updatedTrigger).toBeFocused();

  await updatedTrigger.click();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("heading", { name: "Research settings" }),
  ).toHaveCount(0);
  await expect(updatedTrigger).toBeFocused();
});

test("required response card lines up with the prompt composer", async ({
  page,
}) => {
  await page.goto("/?fixture=interaction-question");

  const interaction = page.locator(
    ".pending-interaction-dock .interaction-card",
  );
  const composer = page.locator(".work-composer");
  await expect(interaction).toBeVisible();
  await expect(composer).toBeVisible();

  const [interactionBox, composerBox] = await Promise.all([
    interaction.boundingBox(),
    composer.boundingBox(),
  ]);
  expect(interactionBox).not.toBeNull();
  expect(composerBox).not.toBeNull();
  expect(Math.abs(interactionBox!.x - composerBox!.x)).toBeLessThan(1);
  expect(
    Math.abs(
      interactionBox!.x +
        interactionBox!.width -
        (composerBox!.x + composerBox!.width),
    ),
  ).toBeLessThan(1);
});

test("follow-ups can be queued, edited, deleted, and used to redirect active work", async ({
  page,
}) => {
  await page.goto("/?fixture=interaction-question");

  const prompt = page.getByLabel("Prompt", { exact: true });
  await expect(prompt).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Add message to Next up" }),
  ).toBeVisible();

  await prompt.fill("Check the Windows path too");
  await page.getByRole("button", { name: "Add message to Next up" }).click();

  const nextUp = page.getByRole("region", { name: "Next up" });
  await expect(nextUp).toContainText("Check the Windows path too");
  await nextUp.getByRole("button", { name: "Edit queued message" }).click();
  const editor = nextUp.getByLabel("Edit queued message");
  await editor.fill("Check Windows and Linux paths");
  await nextUp.getByRole("button", { name: "Save" }).click();
  await expect(nextUp).toContainText("Check Windows and Linux paths");
  await nextUp.getByRole("button", { name: "Delete queued message" }).click();
  await expect(nextUp).toHaveCount(0);

  await prompt.fill("Focus on the cancellation race first");
  await page.getByRole("button", { name: "Redirect current response" }).click();

  await expect(page.getByRole("region", { name: "Next up" })).toHaveCount(0);
  await expect(
    page.getByText("Focus on the cancellation race first", { exact: true }),
  ).toBeVisible();
});

test("conversation follow pauses for reading and resumes on demand or submit", async ({
  page,
}) => {
  const feed = page.locator(".work-feed-scroll");
  await feed.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });

  const jumpToLatest = page.getByRole("button", { name: "Jump to latest" });
  await expect(jumpToLatest).toBeVisible();
  await jumpToLatest.click();
  await expect(jumpToLatest).toHaveCount(0);
  await expect
    .poll(() =>
      feed.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight,
      ),
    )
    .toBeLessThanOrEqual(2);

  await feed.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect(jumpToLatest).toBeVisible();
  await page.getByLabel("Prompt", { exact: true }).fill("Follow this message");
  await page.getByRole("button", { name: "Add message to Next up" }).click();
  await expect(jumpToLatest).toHaveCount(0);
});

test("Colossus responses expose copy confirmation", async ({ page }) => {
  const copyResponse = page
    .getByRole("button", { name: "Copy Colossus response" })
    .first();
  await expect(copyResponse).toBeVisible();
  await copyResponse.click();
  await expect(
    page.getByRole("button", { name: "Copied Colossus response" }).first(),
  ).toBeVisible();
});

test("plan titles open the rendered plan and previews do not leak Markdown syntax", async ({
  page,
}) => {
  await page.goto("/?fixture=plan-workflow");
  await page
    .getByRole("button", { name: "Close details drawer", exact: true })
    .click();
  await page.getByRole("button", { name: "Plans", exact: true }).click();

  const card = page.locator(".session-plan-list article");
  await expect(
    card.getByRole("heading", { name: "Desktop Plan workflow" }),
  ).toBeVisible();
  await expect(card).not.toContainText("## Desktop Plan workflow");

  await card
    .getByRole("button", { name: "Plan the Desktop release workflow" })
    .click();
  const details = page.getByLabel("Thread details", { exact: true });
  await expect(
    details.getByRole("heading", {
      name: "Plan the Desktop release workflow",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    details.getByText("Rendered from the durable plan output"),
  ).toBeVisible();
});

test("Session Map expands canonical resources and opens their inspector", async ({
  page,
}) => {
  const feed = page.locator(".work-feed-scroll");
  await feed.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });

  await page.getByRole("button", { name: "Topology", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Session map", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("form", { name: "Send a prompt" })).toHaveCount(
    0,
  );
  await expect(page.locator(".session-map-primary")).toBeVisible();
  await expect
    .poll(() => feed.evaluate((element) => element.scrollTop))
    .toBe(0);

  const memories = page.getByRole("button", { name: /Memories 3/u });
  await expect(memories).toHaveAttribute("aria-expanded", "true");
  await expect(
    page.getByRole("button", {
      name: /Use Rust 1\.96 and edition 2024/u,
    }),
  ).toBeVisible();

  const goals = page.getByRole("button", { name: /Goals 2/u });
  await goals.click();
  await expect(goals).toHaveAttribute("aria-expanded", "true");
  await expect(
    page.getByRole("button", { name: /Review workspace architecture/u }),
  ).toBeVisible();

  await expect(page.locator(".react-flow")).toBeVisible();
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  await expect(goals).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".session-map-primary")).toBeVisible();

  const feedTopBeforeInspection = await feed.evaluate(
    (element) => element.scrollTop,
  );
  await page
    .getByRole("button", { name: /Use Rust 1\.96 and edition 2024/u })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Use Rust 1.96 and edition 2024 for implementation work.",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByText("Repository", { exact: true })).toBeVisible();
  await expect(page.getByText("100%", { exact: true })).toBeVisible();
  await expect
    .poll(() => feed.evaluate((element) => element.scrollTop))
    .toBe(feedTopBeforeInspection);
});

test("Session Map keeps its root visible without horizontal overflow at compact widths", async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 760 });
  await page.goto(FIXTURE);
  await page.getByRole("button", { name: "Topology", exact: true }).click();

  const stage = page.locator(".session-map-stage");
  const primary = page.locator(".session-map-primary");
  await expect(primary).toBeVisible();
  await expect
    .poll(() =>
      stage.evaluate((element) => element.scrollWidth - element.clientWidth),
    )
    .toBe(0);

  const stageBox = await stage.boundingBox();
  const primaryBox = await primary.boundingBox();
  expect(stageBox).not.toBeNull();
  expect(primaryBox).not.toBeNull();
  expect(primaryBox!.x).toBeGreaterThanOrEqual(stageBox!.x);
  expect(primaryBox!.x + primaryBox!.width).toBeLessThanOrEqual(
    stageBox!.x + stageBox!.width,
  );
});

test("Session Map renders its topology with React Flow SVG edges", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(FIXTURE);
  await page.getByRole("button", { name: "Topology", exact: true }).click();

  const flow = page.locator(".react-flow");
  const primary = page.locator(".session-map-primary");
  const firstFamily = page.locator(".session-map-family").first();
  await expect(flow).toBeVisible();
  await expect(page.locator(".react-flow__edge-path")).toHaveCount(13);
  await expect(page.locator(".session-map-network")).toHaveCount(0);
  await expect(page.locator(".session-map-trunk")).toHaveCount(0);

  const primaryBox = await primary.boundingBox();
  const familyBox = await firstFamily.boundingBox();
  expect(primaryBox).not.toBeNull();
  expect(familyBox).not.toBeNull();
  expect(familyBox!.x).toBeGreaterThan(primaryBox!.x + primaryBox!.width);

  const stageBox = await page.locator(".session-map-stage").boundingBox();
  const flowBox = await flow.boundingBox();
  expect(stageBox).not.toBeNull();
  expect(flowBox).not.toBeNull();
  expect(Math.abs(flowBox!.x - stageBox!.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(flowBox!.y - stageBox!.y)).toBeLessThanOrEqual(1);
  expect(stageBox!.width - flowBox!.width).toBeLessThanOrEqual(2);
  expect(stageBox!.height - flowBox!.height).toBeLessThanOrEqual(2);
});

test("Session Map loads with the native frozen-prototype boundary", async ({
  page,
}) => {
  await page.addInitScript(() => Object.freeze(Object.prototype));
  await page.reload();
  await page.getByRole("button", { name: "Topology", exact: true }).click();

  await expect(page.locator(".react-flow")).toBeVisible();
  await expect(page.getByRole("button", { name: /Goals 2/u })).toBeVisible();
});

test("Session Activity synchronizes timeline, feed, and released inspector content", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${FIXTURE}&view=activity`);

  await expect(
    page.getByRole("heading", { name: "Session activity", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("form", { name: "Send a prompt" })).toHaveCount(
    0,
  );
  const timeline = page.getByRole("region", {
    name: "Session activity timeline",
  });
  await expect(timeline).toBeVisible();
  await expect(timeline.getByText("Agent", { exact: true })).toBeVisible();
  await expect(timeline.getByText("Tools", { exact: true })).toBeVisible();
  await expect(timeline.getByText("System", { exact: true })).toBeVisible();

  const toolRow = page
    .locator(".activity-row", { hasText: "shell.exec" })
    .first();
  await toolRow.click();
  await expect(toolRow).toHaveAttribute("data-selected", "true");
  const inspector = page.getByRole("complementary", {
    name: "Activity inspector",
  });
  await expect(inspector).toContainText("shell.exec");
  await inspector.getByRole("button", { name: "Input", exact: true }).click();
  await expect(inspector.locator("pre")).toContainText("git status --short");

  const timelineTool = page
    .getByRole("button", { name: /filesystem\.search,/u })
    .first();
  await timelineTool.click();
  await expect(inspector).toContainText("filesystem.search");
  await expect(
    page.locator('.activity-row[data-selected="true"]'),
  ).toContainText("filesystem.search");

  const follow = timeline.getByRole("button", {
    name: "Follow live activity",
  });
  await expect(follow).toHaveAttribute("aria-pressed", "true");

  await timeline.getByRole("button", { name: "Zoom in" }).click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");

  await timeline.getByRole("button", { name: "Select time range" }).click();
  const firstTrack = timeline.locator(".activity-timeline-track").first();
  const trackBox = await firstTrack.boundingBox();
  expect(trackBox).not.toBeNull();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.55,
    trackBox!.y + trackBox!.height / 2,
    { steps: 5 },
  );
  await page.mouse.up();
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(3);

  const zoomToRange = timeline.getByRole("button", {
    name: "Zoom to selected time range",
  });
  await expect(zoomToRange).toBeEnabled();
  await zoomToRange.click();
  await follow.click();
  await expect(follow).toHaveAttribute("aria-pressed", "true");

  await timeline.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  await page.keyboard.press("End");
  await expect(follow).toHaveAttribute("aria-pressed", "true");

  const results = await new AxeBuilder({ page })
    .include(".session-activity")
    .analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);
});

test("Session Activity timeline navigation supports mouse, wheel, range, fit, and keyboard input", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${FIXTURE}&view=activity`);

  const timeline = page.getByRole("region", {
    name: "Session activity timeline",
  });
  const axis = timeline.locator(".activity-timeline-axis span");
  const firstTrack = timeline.locator(".activity-timeline-track").first();
  const trackBox = await firstTrack.boundingBox();
  expect(trackBox).not.toBeNull();

  const follow = timeline.getByRole("button", {
    name: "Follow live activity",
  });
  const fit = timeline.getByRole("button", {
    name: "Fit entire timeline",
  });
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await expect(fit).toContainText("1.0×");

  await timeline.getByRole("button", { name: "Zoom in" }).click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  await expect(fit).not.toContainText("1.0×");

  const beforeDragPan = await axis.allTextContents();
  await timeline.getByRole("button", { name: "Pan timeline" }).click();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.68,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.42,
    trackBox!.y + trackBox!.height / 2,
    { steps: 6 },
  );
  await page.mouse.up();
  expect(await axis.allTextContents()).not.toEqual(beforeDragPan);

  await timeline.getByRole("button", { name: "Select time range" }).click();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.76,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.3,
    trackBox!.y + trackBox!.height / 2,
    { steps: 6 },
  );
  await page.mouse.up();
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(3);
  await expect(timeline.getByRole("status")).toContainText("–");

  await timeline.focus();
  await page.keyboard.press("Escape");
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(0);

  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.4,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.4 + 2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.up();
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(0);

  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.55,
    trackBox!.y + trackBox!.height / 2,
    { steps: 5 },
  );
  await page.mouse.up();
  const zoomToRange = timeline.getByRole("button", {
    name: "Zoom to selected time range",
  });
  await zoomToRange.click();
  await expect(fit).not.toContainText("1.0×");
  await fit.click();
  await expect(fit).toContainText("1.0×");
  await expect(
    timeline.getByRole("button", { name: "Zoom out" }),
  ).toBeDisabled();

  await page.mouse.move(
    trackBox!.x + trackBox!.width / 2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -160);
  await page.keyboard.up("Control");
  await expect(fit).not.toContainText("1.0×");

  const beforeWheelPan = await axis.allTextContents();
  await firstTrack.dispatchEvent("wheel", {
    clientX: trackBox!.x + trackBox!.width / 2,
    deltaX: 180,
    deltaY: 0,
  });
  expect(await axis.allTextContents()).not.toEqual(beforeWheelPan);

  await timeline.focus();
  await page.keyboard.press("Home");
  await expect(fit).toContainText("1.0×");
  await page.keyboard.press("+");
  await expect(fit).not.toContainText("1.0×");
  const beforeKeyboardPan = await axis.allTextContents();
  await page.keyboard.press("ArrowLeft");
  expect(await axis.allTextContents()).not.toEqual(beforeKeyboardPan);
  await page.keyboard.press("End");
  await expect(follow).toHaveAttribute("aria-pressed", "true");
});

test("Session Activity Follow tracks new records and manual navigation holds its viewport", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${FIXTURE}&view=activity&activityLive=1`);

  const timeline = page.getByRole("region", {
    name: "Session activity timeline",
  });
  const axis = timeline.locator(".activity-timeline-axis span");
  const follow = timeline.getByRole("button", {
    name: "Follow live activity",
  });
  const initialAxis = await axis.allTextContents();
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByText("27 curated events", { exact: true }),
  ).toBeVisible();

  await expect(
    timeline.getByRole("button", { name: /Live checkpoint,/u }),
  ).toBeVisible({ timeout: 7_000 });
  await expect(
    page.getByText("28 curated events", { exact: true }),
  ).toBeVisible();
  expect(await axis.allTextContents()).not.toEqual(initialAxis);

  await timeline.getByRole("button", { name: "Zoom in" }).click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  const heldAxis = await axis.allTextContents();

  await expect(
    page.getByText("29 curated events", { exact: true }),
  ).toBeVisible({ timeout: 7_000 });
  await expect(
    timeline.getByRole("button", { name: /Live response,/u }),
  ).toHaveCount(0);
  expect(await axis.allTextContents()).toEqual(heldAxis);

  await follow.click();
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await expect(
    timeline.getByRole("button", { name: /Live response,/u }),
  ).toBeVisible();
  expect(await axis.allTextContents()).not.toEqual(heldAxis);
});

test("Session Activity search, filters, live state, and compact stacking remain functional", async ({
  page,
}) => {
  await page.setViewportSize({ width: 740, height: 780 });
  await page.goto(`${FIXTURE}&view=activity`);

  const search = page.getByRole("searchbox", {
    name: "Search session activity",
  });
  await search.fill("denied");
  await expect(page.locator(".activity-row")).toHaveCount(1);
  await expect(page.locator(".activity-row")).toContainText("denied");

  await page.getByRole("button", { name: "Filter", exact: true }).click();
  const filters = page.getByRole("dialog", { name: "Activity filters" });
  await filters.getByRole("checkbox", { name: "Tools", exact: true }).check();
  await filters.getByRole("checkbox", { name: "Failed", exact: true }).check();
  await expect(page.getByRole("button", { name: "Filter (2)" })).toBeVisible();
  await expect(page.locator(".activity-row")).toHaveCount(1);

  const live = page.getByRole("button", { name: "Live", exact: true });
  await live.click();
  await expect(
    page.getByRole("button", { name: "Paused", exact: true }),
  ).toHaveAttribute("aria-pressed", "false");

  const feedBox = await page
    .getByRole("region", { name: "Session activity feed" })
    .boundingBox();
  const inspectorBox = await page
    .getByRole("complementary", { name: "Activity inspector" })
    .boundingBox();
  expect(feedBox).not.toBeNull();
  expect(inspectorBox).not.toBeNull();
  expect(inspectorBox!.y).toBeGreaterThanOrEqual(feedBox!.y + feedBox!.height);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    ),
  ).toBe(0);
});

test("right-side drawers trap focus, close with Escape, and restore focus", async ({
  page,
}) => {
  const filesTrigger = page.getByRole("button", { name: "Open files panel" });
  await filesTrigger.click();

  const filesDialog = page.getByRole("dialog", { name: "Workspace files" });
  await expect(filesDialog).toBeVisible();
  await expect(
    filesDialog.getByRole("button", { name: "Close files drawer" }),
  ).toBeFocused();

  await page.keyboard.press("Shift+Tab");
  await expect(filesDialog).toContainText("Read-only");
  await page.keyboard.press("Escape");
  await expect(filesDialog).toHaveCount(0);
  await expect(filesTrigger).toBeFocused();

  const artifactsTrigger = page.getByRole("button", {
    name: /Open artifacts panel, 3 artifacts/u,
  });
  await artifactsTrigger.click();
  const artifactsDialog = page.getByRole("dialog", {
    name: "Artifact preview",
  });
  await expect(artifactsDialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(artifactsDialog).toHaveCount(0);
  await expect(artifactsTrigger).toBeFocused();
});

test("Workspace navigation and approvals are keyboard-operable", async ({
  page,
}) => {
  const navigationTrigger = page.getByRole("button", {
    name: "Open work navigation",
  });
  await navigationTrigger.click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  await expect(navigation).toBeVisible();
  await expect(
    navigation.getByRole("button", { name: "Close navigation" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(navigation).toHaveCount(0);
  await expect(navigationTrigger).toBeFocused();

  await expect(
    page.getByRole("heading", {
      name: "Apply the hardened bootstrap changes",
    }),
  ).toBeVisible();
  const allow = page.getByRole("button", { name: "Allow once" });
  await allow.focus();
  await page.keyboard.press("Enter");
  await expect(allow).toHaveCount(0);
  await expect(page.getByLabel("Required response")).toHaveCount(0);
});

test("responsive Workspace navigation remains reachable from catalog surfaces", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Capabilities", exact: true })
    .click();

  const navigationTrigger = page.getByRole("button", {
    name: "Open Workspace navigation",
  });
  await expect(navigationTrigger).toBeVisible();
  await navigationTrigger.click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  const close = navigation.getByRole("button", { name: "Close navigation" });
  await expect(close).toBeVisible();
  await close.click();
  await expect(navigation).toHaveCount(0);
  await expect(navigationTrigger).toBeFocused();
});

test("desktop Workspace sidebar resizes and remembers its width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const sidebar = page.locator(".work-sidebar");
  const resizeHandle = page.getByRole("separator", {
    name: "Resize Workspace sidebar",
  });
  const initialBox = await sidebar.boundingBox();
  const handleBox = await resizeHandle.boundingBox();
  expect(initialBox).not.toBeNull();
  expect(handleBox).not.toBeNull();

  await page.mouse.move(
    handleBox!.x + handleBox!.width / 2,
    handleBox!.y + handleBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(handleBox!.x + 84, handleBox!.y + 120, { steps: 5 });
  await page.mouse.up();

  await expect
    .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
    .toBeGreaterThan(initialBox!.width + 60);
  const resizedWidth = (await sidebar.boundingBox())!.width;

  await page.reload();
  await expect
    .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
    .toBeCloseTo(resizedWidth, 0);

  await resizeHandle.focus();
  await page.keyboard.press("ArrowLeft");
  await expect
    .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
    .toBeCloseTo(resizedWidth - 8, 0);
});

test("collapsing the active Workspace keeps the remaining navigation pinned", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const sidebar = page.locator(".work-sidebar");
  const threadStack = page.locator(".space-thread-stack");
  const footer = page.locator(".space-sidebar-footer");
  const footerBefore = await footer.boundingBox();
  const sidebarBox = await sidebar.boundingBox();
  expect(footerBefore).not.toBeNull();
  expect(sidebarBox).not.toBeNull();

  await page.getByRole("button", { name: "Collapse Colossus threads" }).click();

  await expect(threadStack).toHaveAttribute("aria-hidden", "true");
  await expect(
    page.getByRole("button", { name: "Expand Colossus threads" }),
  ).toBeVisible();
  await expect
    .poll(async () => (await footer.boundingBox())?.y ?? 0)
    .toBeCloseTo(footerBefore!.y, 0);

  const footerAfter = await footer.boundingBox();
  expect(footerAfter).not.toBeNull();
  expect(
    sidebarBox!.y + sidebarBox!.height - (footerAfter!.y + footerAfter!.height),
  ).toBeLessThanOrEqual(14);
});

test("Workspace folders disclose thread summaries without switching context", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const activeSpace = page.locator(".space-shelf.is-active");
  await expect(activeSpace).toContainText("Colossus");

  await page
    .getByRole("button", { name: "Expand Research Lab threads" })
    .click();
  await expect(page.getByText("Review source provenance")).toBeVisible();
  await expect(activeSpace).toContainText("Colossus");

  await page
    .getByRole("button", { name: "Expand Proposal Studio threads" })
    .click();
  await expect(page.getByText("Resolve compliance findings")).toBeVisible();
  await expect(page.getByText("Review source provenance")).toBeVisible();

  await page.getByRole("button", { name: "Research Lab", exact: true }).click();
  await expect(activeSpace).toContainText("Research Lab");
  await expect(
    page.getByRole("button", { name: "New thread in Research Lab" }),
  ).toBeVisible();
});

test("selected Workspace keeps its create action on the trailing edge", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const activeSpace = page.locator(".space-shelf.is-active");
  const disclosure = activeSpace.getByRole("button", {
    name: "Collapse Colossus threads",
  });
  const create = activeSpace.getByRole("button", {
    name: "New thread in Colossus",
  });
  const [disclosureBox, createBox] = await Promise.all([
    disclosure.boundingBox(),
    create.boundingBox(),
  ]);

  expect(disclosureBox).not.toBeNull();
  expect(createBox).not.toBeNull();
  expect(createBox!.x).toBeGreaterThan(disclosureBox!.x + disclosureBox!.width);
});

test("terminal threads can be archived from the Workspace sidebar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  const archive = page.getByRole("button", {
    name: "Archive Audit ipc boundary",
  });
  await expect(archive).toBeEnabled();
  await archive.click();

  await expect(archive).toHaveCount(0);
  await expect(
    page.getByText("Audit ipc boundary", { exact: true }),
  ).toHaveCount(0);
});

test("threads can be pinned, persisted, and returned to their normal group", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  await page.getByText("Audit ipc boundary", { exact: true }).hover();
  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  await page.getByRole("button", { name: "Pin Audit ipc boundary" }).click();

  const pinned = page.locator(".work-group").filter({
    has: page.getByRole("heading", { name: "Pinned", exact: true }),
  });
  await expect(pinned).toContainText("Audit ipc boundary");
  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Unpin Audit ipc boundary" }),
  ).toHaveAttribute("aria-pressed", "true");

  await page.reload();
  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Unpin Audit ipc boundary" }),
  ).toBeAttached();

  await page.getByRole("button", { name: "Unpin Audit ipc boundary" }).click();
  await expect(pinned).toContainText("Harden desktop agent bootstrap");
  await expect(pinned).not.toContainText("Audit ipc boundary");

  await page
    .getByRole("button", {
      name: "Thread actions for Harden desktop agent bootstrap",
    })
    .click();
  await page
    .getByRole("button", { name: "Unpin Harden desktop agent bootstrap" })
    .click();
  await expect(pinned).toHaveCount(0);

  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Pinned", exact: true }),
  ).toHaveCount(0);
});

test("pinning remains available while the startup connection settles", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(`${FIXTURE}&connecting=1`);

  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  const pin = page.getByRole("button", { name: "Pin Audit ipc boundary" });
  await expect(pin).toBeEnabled();
  await pin.click();

  const pinned = page.locator(".work-group").filter({
    has: page.getByRole("heading", { name: "Pinned", exact: true }),
  });
  await expect(pinned).toContainText("Audit ipc boundary");
});

test("thread actions survive a WebKit blur without a related target", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });

  const actions = page.getByRole("button", {
    name: "Thread actions for Audit ipc boundary",
  });
  await actions.click();
  const pin = page.getByRole("button", { name: "Pin Audit ipc boundary" });
  await pin.evaluate((button) => {
    const menu = button.closest("details");
    const summary = menu?.querySelector("summary") ?? null;
    if (menu === null || summary === null) {
      throw new Error("thread actions menu is unavailable");
    }
    summary.dispatchEvent(
      new FocusEvent("blur", { bubbles: true, relatedTarget: null }),
    );
    button.focus();
  });
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );

  await expect(pin).toBeVisible();
  await pin.click();
  const pinned = page.locator(".work-group").filter({
    has: page.getByRole("heading", { name: "Pinned", exact: true }),
  });
  await expect(pinned).toContainText("Audit ipc boundary");
});

test("search scope is part of the search control", async ({ page }) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  const search = navigation.getByRole("searchbox", {
    name: "Search threads",
  });
  const scope = navigation.getByLabel("Thread search scope");

  await expect(search).toHaveAttribute("placeholder", "Search threads");
  await expect(
    scope.getByRole("button", { name: "This Workspace", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");

  await scope
    .getByRole("button", { name: "All Workspaces", exact: true })
    .click();

  await expect(
    scope.getByRole("button", { name: "All Workspaces", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    scope.getByRole("button", { name: "This Workspace", exact: true }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(search).toBeFocused();
});

test("Thread details lists released participants and returns focus on close", async ({
  page,
}) => {
  const detailsTrigger = page.getByRole("button", {
    name: "Open thread details",
  });
  await detailsTrigger.click();

  const details = page.getByRole("dialog", { name: "Thread details" });
  await expect(details).toBeVisible();
  await expect(details).toContainText("Atlas");
  await expect(details).toContainText("Builder");
  await expect(details).toContainText("Sentinel");
  await expect(details).toContainText("Scribe");
  await expect(details).toContainText("bootstrap.rs");

  await details.getByRole("button", { name: "Close details drawer" }).click();
  await expect(details).toHaveCount(0);
  await expect(detailsTrigger).toBeFocused();
});

test("Workspace startup keeps search and navigation responsive", async ({
  page,
}) => {
  await page.goto(`${FIXTURE}&spaceStartup=1`);
  await page.getByRole("button", { name: "Open work navigation" }).click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  await expect(
    navigation.getByText("Research Lab", { exact: true }),
  ).toBeVisible();
  await expect(navigation.getByText("Starting", { exact: true })).toBeVisible();
  await expect(
    navigation.getByRole("searchbox", { name: "Search threads" }),
  ).toBeEnabled();
  await expect(
    navigation.getByRole("button", { name: "Capabilities", exact: true }),
  ).toBeEnabled();
  await expect(
    navigation.getByRole("button", { name: "New thread in Research Lab" }),
  ).toBeDisabled();
  await expect(
    navigation.locator('.space-shelf.is-active[aria-busy="true"]'),
  ).toBeVisible();
  await expect(
    navigation.locator('.space-shelf-identity[title="~/tools/research-lab"]'),
  ).toBeVisible();
});

test("follow-up prompts remain in the same work conversation", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "New thread in Colossus" })
    .click();

  const prompt = page.getByRole("textbox", { name: "Prompt" });
  const opening = "Review the updater configuration";
  const followUp = "Now check the Windows preview path";
  await prompt.fill(opening);
  await prompt.press("Enter");
  await expect(page.locator(".message-user .message-body")).toContainText(
    opening,
  );

  await prompt.fill(followUp);
  await prompt.press("Enter");
  await expect(page.locator(".message-user .message-body")).toHaveCount(2);
  await expect(
    page.locator(".message-user .message-body").nth(0),
  ).toContainText(opening);
  await expect(
    page.locator(".message-user .message-body").nth(1),
  ).toContainText(followUp);
  await expect(page.locator(".message-assistant")).toHaveCount(2);
  await expect(page.getByRole("heading", { name: opening })).toBeVisible();

  await page.getByRole("button", { name: "Open work navigation" }).click();
  const workNavigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  await expect(
    workNavigation.locator(".work-item").filter({ hasText: opening }),
  ).toHaveCount(1);
  await expect(workNavigation.getByText(followUp, { exact: true })).toHaveCount(
    0,
  );
});

test("high-contrast mode preserves visible focus and controls", async ({
  page,
}) => {
  await page.emulateMedia({ forcedColors: "active" });
  const filesTrigger = page.getByRole("button", { name: "Open files panel" });
  await filesTrigger.focus();
  await expect(filesTrigger).toBeFocused();
  await expect(filesTrigger).toBeVisible();

  const focusStyle = await filesTrigger.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
    };
  });
  expect(focusStyle.outlineStyle).not.toBe("none");
  expect(focusStyle.outlineWidth).not.toBe("0px");
});
