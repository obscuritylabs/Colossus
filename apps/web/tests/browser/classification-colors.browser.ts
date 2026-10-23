import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Classification } from "../../src/control-api";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

const marking: Classification = {
  enabled: true,
  text: "INTERNAL · TEST ENVIRONMENT",
  tone: "warning",
  position: "top_and_bottom",
};

async function color(input: Locator, value: string) {
  // The operating system owns the native color-picker dialog. Exercise its input event.
  await input.evaluate((element, next) => {
    const input = element as HTMLInputElement;
    input.value = next;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
}

async function emptyAnnouncements(page: Page) {
  await page.route("**/api/announcements**", (route) =>
    route.fulfill({
      json: { active_banner: null, announcements: [], next_cursor: null },
    }),
  );
}

test("global administrators publish and reset readable classification colors from Appearance", async ({
  page,
}, testInfo) => {
  const fixture = await browserFixture(page, { globalAdmin: true });
  await emptyAnnouncements(page);
  let classification = { ...marking };
  let revision = 4;
  const changes: Classification[] = [];
  await page.route("**/api/settings", (route) =>
    route.fulfill({ json: { revision, classification } }),
  );
  await page.route("**/api/admin/settings", async (route) => {
    if (route.request().method() === "PATCH") {
      expect(route.request().headers()["x-colossus-csrf"]).toBe("1");
      const body = route.request().postDataJSON() as {
        revision: number;
        classification: Classification;
      };
      expect(body.revision).toBe(revision);
      expect({ ...body.classification, colors: undefined }).toEqual({
        ...marking,
        colors: undefined,
      });
      classification = body.classification;
      changes.push(classification);
      revision++;
      await route.fulfill({ json: { revision, classification } });
    } else {
      await route.fulfill({
        json: {
          revision,
          classification,
          auth: { local_enabled: true, oidc_label: null },
        },
      });
    }
  });
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/admin/appearance?project=browser-project");
  const editor = page.getByRole("region", {
    name: "Classification banner colors",
    exact: true,
  });
  const publish = editor.getByRole("button", {
    name: "Publish banner colors",
    exact: true,
  });
  await expect(editor).toBeVisible();
  await color(
    editor.getByLabel("Classification background color", { exact: true }),
    "#ffffff",
  );
  await expect(publish).toBeDisabled();
  await expect(editor.getByRole("status")).toContainText(
    "at least 4.5:1 contrast before applying",
  );
  await color(
    editor.getByLabel("Classification background color", { exact: true }),
    "#008000",
  );
  await expect(
    editor.getByLabel("Classification color preview", { exact: true }),
  ).toHaveCSS("background-color", "rgb(0, 128, 0)");
  await publish.focus();
  await page.keyboard.press("Enter");
  const banners = page.locator(
    ".control-plane-frame > .control-classification",
  );
  await expect(banners).toHaveCount(2);
  for (const banner of await banners.all()) {
    await expect(banner).toHaveCSS("background-color", "rgb(0, 128, 0)");
    await expect(banner).toHaveCSS("color", "rgb(255, 255, 255)");
    await expect(banner).toHaveText(marking.text);
  }
  await editor.screenshot({
    path: testInfo.outputPath("classification-colors-admin.png"),
  });
  await page.setViewportSize({ width: 390, height: 844 });
  for (const appearance of appearanceCases) {
    await selectAppearance(page, appearance);
    await page.goto("/admin/appearance?project=browser-project");
    await expect(editor).toBeVisible();
    await expect(banners.first()).toHaveCSS(
      "background-color",
      "rgb(0, 128, 0)",
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`appearance-${appearance.id}-large-mobile.png`),
      fullPage: true,
    });
  }
  await editor.screenshot({
    path: testInfo.outputPath("classification-colors-large-mobile.png"),
  });
  const reset = editor.getByRole("button", {
    name: "Use tone colors",
    exact: true,
  });
  await reset.focus();
  await page.keyboard.press("Enter");
  await expect(reset).toBeDisabled();
  await expect(banners.first()).not.toHaveClass(/classification-custom-colors/);
  await expect(banners.first()).not.toHaveCSS(
    "background-color",
    "rgb(0, 128, 0)",
  );
  expect(changes).toHaveLength(2);
  expect(changes[1]!.colors).toBeNull();
  fixture.verify();
});

test("ordinary users see published classification colors without editing global Appearance", async ({
  page,
}) => {
  const fixture = await browserFixture(page);
  await emptyAnnouncements(page);
  await page.route("**/api/settings", (route) =>
    route.fulfill({
      json: {
        classification: {
          ...marking,
          colors: { background: "#008000", text: "#ffffff" },
        },
      },
    }),
  );
  const adminReads: string[] = [];
  await page.route("**/api/admin/settings", (route) => {
    adminReads.push(route.request().method());
    return route.fulfill({ status: 403, json: { error: "permission_denied" } });
  });
  await page.goto("/settings?project=browser-project");
  await expect(
    page.getByRole("heading", { name: "Appearance", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", {
      name: "Classification banner colors",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.locator(".control-plane-frame > .control-classification").first(),
  ).toHaveCSS("background-color", "rgb(0, 128, 0)");
  expect(adminReads).toEqual([]);
  fixture.verify();
});

test("both public sign-in classification banners use the published color pair", async ({
  page,
}, testInfo) => {
  const unexpected: string[] = [];
  await page.route("**/api/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/me")
      return route.fulfill({
        status: 403,
        json: { error: "permission_denied" },
      });
    if (path === "/api/auth/config")
      return route.fulfill({ json: { local_enabled: true, oidc: null } });
    if (path === "/api/settings")
      return route.fulfill({
        json: {
          classification: {
            ...marking,
            colors: { background: "#008000", text: "#ffffff" },
          },
        },
      });
    unexpected.push(path);
    return route.fulfill({ status: 404, json: { error: "not_found" } });
  });
  await page.goto("/settings");
  await expect(
    page.getByRole("heading", {
      name: "Sign in to your Control Plane",
      exact: true,
    }),
  ).toBeVisible();
  const banners = page.locator(".login-layout > .control-classification");
  await expect(banners).toHaveCount(2);
  for (const banner of await banners.all()) {
    await expect(banner).toHaveText(marking.text);
    await expect(banner).toHaveCSS("background-color", "rgb(0, 128, 0)");
    await expect(banner).toHaveCSS("color", "rgb(255, 255, 255)");
  }
  await page.screenshot({
    path: testInfo.outputPath("classification-colors-sign-in.png"),
    fullPage: true,
  });
  expect(unexpected).toEqual([]);
});

for (const appearance of appearanceCases) {
  test(`personal Appearance keeps readable copy and controls in ${appearance.label} Large layouts`, async ({
    page,
  }, testInfo) => {
    const fixture = await browserFixture(page);
    await selectAppearance(page, appearance);
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      const copies = page.locator(".appearance-control-copy");
      await expect(copies).toHaveCount(3);
      for (const copy of await copies.all()) {
        const bounds = await copy.boundingBox();
        expect(bounds?.width).toBeGreaterThanOrEqual(180);
        const metrics = await copy.locator("small").evaluate((element) => ({
          height: element.getBoundingClientRect().height,
          lineHeight: Number.parseFloat(getComputedStyle(element).lineHeight),
        }));
        expect(metrics.height).toBeLessThanOrEqual(metrics.lineHeight * 6);
      }
      for (const name of [
        "appearance-color-theme",
        "appearance-dark-palette",
        "appearance-text-size",
      ]) {
        const control = page.locator(`#${name}`);
        await control.scrollIntoViewIfNeeded();
        await expect(control).toBeInViewport();
        const bounds = await control.boundingBox();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      }
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: testInfo.outputPath(
          `personal-appearance-${appearance.id}-${width}.png`,
        ),
        fullPage: true,
      });
    }
    fixture.verify();
  });
}
