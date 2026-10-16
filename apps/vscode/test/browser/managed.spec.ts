import { expect, test } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { DEFAULT_PREFERENCES, type SettingsView } from "../../src/settings.js";

test("Managed Local is the default sidebar action without enrollment inputs", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const host = window as unknown as {
      actions: unknown[];
      acquireVsCodeApi: () => unknown;
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      postMessage: (value: unknown) => host.actions.push(value),
      getState: () => undefined,
      setState() {},
    });
  });
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Start Colossus", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Start Colossus", exact: true })
    .click();
  expect(
    await page.evaluate(
      () => (window as unknown as { actions: unknown[] }).actions,
    ),
  ).toContainEqual({ type: "connect" });
  await expect(page.locator("input[type=password]")).toHaveCount(0);
});

test("managed settings expose native setup and remain usable across palettes and compact layouts", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const host = window as unknown as {
      actions: unknown[];
      acquireVsCodeApi: () => unknown;
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      postMessage: (value: unknown) => host.actions.push(value),
      getState: () => ({ scope: "workspace", section: "connections" }),
      setState() {},
    });
  });
  await page.goto("/settings");
  const view: SettingsView = {
    preferences: DEFAULT_PREFERENCES,
    preferenceError: "",
    workspace: "managed project",
    connected: false,
    connecting: false,
    busy: false,
    hasSavedConnection: false,
    version: "",
    role: "",
    error: "",
    runtimeKind: "managed",
  };
  for (const palette of ["editor", "colossus", "black", "hacker"] as const) {
    await page.setViewportSize({ width: 260, height: 900 });
    await page.evaluate(
      (view) => window.postMessage({ type: "settings", view }, "*"),
      { ...view, preferences: { ...view.preferences, palette } },
    );
    await page
      .locator("html")
      .evaluate((root) => root.style.setProperty("--font-size-body", "18px"));
    const configure = page.getByRole("button", {
      name: "Configure Local Runtime",
      exact: true,
    });
    await expect(configure).toBeVisible();
    await configure.focus();
    await expect(configure).toBeFocused();
    await configure.press("Enter");
    expect(
      await page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    ).toContainEqual({ type: "configureManaged" });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    const axe = await new AxeBuilder({ page }).analyze();
    expect(axe.violations).toEqual([]);
  }
  await page.setViewportSize({ width: 1000, height: 800 });
  await page.screenshot({
    path: "artifacts/managed-local-settings.png",
    fullPage: true,
  });
});
