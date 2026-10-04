import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { DEFAULT_PREFERENCES, type SettingsView } from "../../src/settings.js";

async function choose(page: Page, label: string, option: string) {
  await page.getByRole("combobox", { name: label, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

test("desktop-style settings navigate, search and request validated host changes", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await page.addInitScript(() => {
    const host = window as unknown as {
      acquireVsCodeApi: () => unknown;
      actions: unknown[];
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      postMessage: (action: unknown) => host.actions.push(action),
      getState: () => undefined,
      setState: () => undefined,
    });
  });
  await page.goto("/settings");
  const view: SettingsView = {
    preferences: DEFAULT_PREFERENCES,
    workspace: "Colossus",
    connected: true,
    connecting: false,
    busy: false,
    hasSavedConnection: true,
    version: "0.11.7",
    role: "primary",
    error: "",
  };
  await page.evaluate(
    (view) => window.postMessage({ type: "settings", view }, "*"),
    view,
  );
  await expect(page.locator("html")).toHaveAttribute("data-palette", "neutral");
  await choose(page, "Surface palette", "Colossus blue");
  await expect(page.locator("html")).toHaveAttribute(
    "data-palette",
    "colossus",
  );
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "setPreference",
      name: "palette",
      value: "colossus",
    });
  await choose(page, "Surface palette", "Hacker (TUI)");
  await expect(page.locator("html")).toHaveAttribute("data-palette", "hacker");
  await expect
    .poll(() =>
      page
        .locator("html")
        .evaluate((root) =>
          getComputedStyle(root).getPropertyValue("--blue").trim(),
        ),
    )
    .toBe("#00ff66");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "setPreference",
      name: "palette",
      value: "hacker",
    });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: "artifacts/settings-hacker.png" });
  await choose(page, "Surface palette", "Editor (Dark+)");
  await expect(page.locator("html")).toHaveAttribute("data-palette", "neutral");
  await page.getByRole("button", { name: "Defaults", exact: true }).click();
  await choose(page, "Send shortcut", "Enter");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "setPreference",
      name: "sendShortcut",
      value: "enter",
    });
  await choose(page, "Default run mode", "Execute");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "setPreference",
      name: "defaultMode",
      value: "execute",
    });
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await page
    .getByRole("button", { name: "Credential location", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({ type: "configureCredential" });
  await expect(
    page.getByRole("button", { name: "Disconnect", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Show diagnostics" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({ type: "showConnectionLog" });
  await page.getByRole("button", { name: "Runtime", exact: true }).click();
  await expect(page.getByText("0.11.7", { exact: true })).toBeVisible();
  await expect(page.getByText("primary", { exact: true })).toBeVisible();
  await page
    .getByRole("searchbox", { name: "Search settings" })
    .fill("send shortcut");
  await expect(page.getByLabel("Send shortcut", { exact: true })).toBeVisible();
  await page.getByRole("searchbox").fill("no match xyz");
  await expect(page.getByText("No matching settings.")).toBeVisible();
  await page.getByRole("searchbox").fill("");
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("switch", { name: "Show tool activity" }).uncheck();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "setPreference",
      name: "showToolActivity",
      value: false,
    });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({ path: "artifacts/settings-appearance.png" });
  await page.getByRole("button", { name: "Defaults", exact: true }).click();
  await page.screenshot({ path: "artifacts/settings-defaults.png" });
  await page.setViewportSize({ width: 360, height: 850 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await page.evaluate(
    (view) =>
      window.postMessage(
        { type: "settings", view: { ...view, busy: true } },
        "*",
      ),
    view,
  );
  await expect(
    page.getByRole("button", { name: "Disconnect", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Forget connection" }),
  ).toBeDisabled();
});
