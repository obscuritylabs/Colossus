import { expect, test } from "@playwright/test";

test("Git and Browser preferences change their panes and survive Settings navigation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  const sections = page.getByRole("navigation", { name: "Settings sections" });
  await sections.getByRole("button", { name: "Git", exact: true }).click();
  await page
    .getByRole("switch", { name: "Automatically refresh Git" })
    .uncheck();
  await page.getByRole("combobox", { name: "Opening view" }).click();
  await page.getByRole("option", { name: "History", exact: true }).click();

  await sections.getByRole("button", { name: "Browser", exact: true }).click();
  await page
    .getByRole("textbox", { name: "New-tab page" })
    .fill("javascript:alert(1)");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "full http:// or https:// URL",
  );
  await page
    .getByRole("textbox", { name: "New-tab page" })
    .fill("https://example.com/docs");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "New-tab page" })).toHaveValue(
    "https://example.com/docs",
  );

  await sections.getByRole("button", { name: "Terminal", exact: true }).click();
  const terminalSwitch = page.getByRole("switch", {
    name: "Enable local terminal",
  });
  await expect(terminalSwitch).toBeChecked();
  await expect(page.getByRole("button", { name: "Open Shell" })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Open Colossus TUI" }),
  ).toHaveCount(0);
  await terminalSwitch.uncheck();
  await expect(terminalSwitch).not.toBeChecked();
  await terminalSwitch.check();
  await page.getByRole("combobox", { name: "Default session" }).click();
  await page.getByRole("option", { name: "System shell" }).click();
  await expect(
    page.getByRole("combobox", { name: "Default session" }),
  ).toContainText("System shell");

  await sections
    .getByRole("button", { name: "Appearance", exact: true })
    .click();
  await page.getByRole("combobox", { name: /^Color theme/u }).click();
  await page.getByRole("option", { name: "Light", exact: true }).click();
  await expect(page.getByLabel("light theme preview")).toBeVisible();

  await page.getByRole("button", { name: "Back to work" }).click();
  await page.getByRole("button", { name: /Open Git:/ }).click();
  const git = page.getByRole("region", { name: "Workspace Git" });
  await expect(
    git.getByRole("tab", { name: "History", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(git).toContainText("Manual refresh");
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
  await page.getByRole("button", { name: "New browser tab" }).click();
  await expect(page.getByRole("textbox", { name: "Web address" })).toHaveValue(
    "https://example.com/docs",
  );
});

test("a new workspace can confirm default terminal access on first use", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio&terminalConsentPending=1");
  await page.getByRole("button", { name: "Manage Workspaces" }).click();
  const openShell = page.getByRole("button", { name: "Open Shell" });
  await expect(openShell).toBeEnabled();
  await openShell.click();
  await expect(
    page.getByRole("region", { name: "Terminal pane" }),
  ).toBeVisible();
});
