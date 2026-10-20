import { expect, test, type Locator, type Page } from "@playwright/test";

type Insets = { top: number; left: number; right: number; bottom: number };
const portrait = {
  viewport: { width: 390, height: 844 },
  insets: { top: 48, left: 14, right: 76, bottom: 34 },
};
const landscape = {
  viewport: { width: 844, height: 390 },
  insets: { top: 16, left: 66, right: 22, bottom: 24 },
};
const narrow = {
  viewport: { width: 320, height: 844 },
  insets: { top: 54, left: 52, right: 8, bottom: 28 },
};

async function safeAreaSession(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  try {
    await cdp.send("Emulation.setSafeAreaInsetsOverride", {
      insets: portrait.insets,
    });
  } catch (failure) {
    await cdp.detach();
    test.skip(
      String(failure).includes("wasn't found"),
      "This Chromium version does not implement safe-area emulation.",
    );
    throw failure;
  }
  return cdp;
}

async function unobscured(
  control: Locator,
  viewport: { width: number; height: number },
  insets: Insets,
) {
  await expect(control).toBeInViewport({ ratio: 1 });
  const bounds = (await control.boundingBox())!;
  // Browsers round scroll offsets to device pixels while layout can be fractional.
  const rounding = 1;
  expect(bounds.x).toBeGreaterThanOrEqual(insets.left - rounding);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(
    viewport.width - insets.right + rounding,
  );
  expect(bounds.y).toBeGreaterThanOrEqual(insets.top - rounding);
  expect(bounds.y + bounds.height).toBeLessThanOrEqual(
    viewport.height - insets.bottom + rounding,
  );
}

test("Large login controls avoid asymmetric cutouts in portrait, landscape and narrow layouts", async ({
  page,
  browserName,
}) => {
  test.skip(
    browserName !== "chromium",
    "Safe-area emulation uses Chromium CDP.",
  );
  const cdp = await safeAreaSession(page);
  await page.addInitScript(() =>
    localStorage.setItem("colossus.mobile.text-size", "large"),
  );
  try {
    for (const { viewport, insets } of [portrait, landscape, narrow]) {
      await page.setViewportSize(viewport);
      await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets });
      await page.goto("/");
      const address = page.getByRole("textbox", {
        name: "Server address",
        exact: true,
      });
      await address.evaluate((element) =>
        element.scrollIntoView({ block: "center" }),
      );
      await unobscured(address, viewport, insets);
      const preview = page.getByRole("button", {
        name: "Explore development preview",
        exact: true,
      });
      await preview.evaluate((element) =>
        element.scrollIntoView({ block: "center" }),
      );
      await unobscured(preview, viewport, insets);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    }
  } finally {
    await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: {} });
    await cdp.detach();
  }
});

test("Large task controls and portalled options remain usable as safe areas rotate", async ({
  page,
  browserName,
}) => {
  test.skip(
    browserName !== "chromium",
    "Safe-area emulation uses Chromium CDP.",
  );
  const cdp = await safeAreaSession(page);
  await page.addInitScript(() =>
    localStorage.setItem("colossus.mobile.text-size", "large"),
  );
  try {
    await page.setViewportSize(portrait.viewport);
    await cdp.send("Emulation.setSafeAreaInsetsOverride", {
      insets: portrait.insets,
    });
    await page.goto("/");
    await page
      .getByRole("button", { name: "Explore development preview", exact: true })
      .click();
    await unobscured(
      page.getByRole("combobox", { name: "Project", exact: true }),
      portrait.viewport,
      portrait.insets,
    );
    const create = page.getByRole("button", { name: "New task", exact: true });
    await unobscured(create, portrait.viewport, portrait.insets);
    await create.click();
    const input = page.getByRole("textbox", {
      name: "Message Colossus",
      exact: true,
    });
    await input.fill("Check the rotated safe area");
    for (const { viewport, insets } of [portrait, landscape, narrow]) {
      await page.setViewportSize(viewport);
      await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets });
      await unobscured(input, viewport, insets);
      const options = page.getByRole("button", {
        name: "Task options · Execute",
        exact: true,
      });
      await options.click();
      const close = page.locator(".mobile-task-options .ui-sheet-close-button");
      await unobscured(close, viewport, insets);
      const done = page.getByRole("button", { name: "Done", exact: true });
      await unobscured(done, viewport, insets);
      await expect(
        page.getByRole("dialog", { name: "Task options", exact: true }),
      ).toBeVisible();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      for (const name of ["Plan", "Execute", "Goal", "Research"]) {
        const mode = page
          .locator(".mode-switch label")
          .filter({ hasText: name });
        await mode.scrollIntoViewIfNeeded();
        await unobscured(mode, viewport, insets);
      }
      await page.screenshot({
        path: `test-results/safe-area-options-${viewport.width}.png`,
      });
      await close.click();
      await expect(options).toBeFocused();
      await expect(input).toHaveValue("Check the rotated safe area");
    }
    await unobscured(
      page.getByRole("button", { name: "Send", exact: true }),
      narrow.viewport,
      narrow.insets,
    );
    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect(
      page.getByRole("heading", {
        name: "Check the rotated safe area",
        exact: true,
      }),
    ).toBeVisible();
  } finally {
    await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: {} });
    await cdp.detach();
  }
});

test("released artifact details and modal dismissal avoid rotating cutouts with Large text", async ({
  page,
  browserName,
}) => {
  test.skip(
    browserName !== "chromium",
    "Safe-area emulation uses Chromium CDP.",
  );
  const cdp = await safeAreaSession(page);
  await page.addInitScript(() =>
    localStorage.setItem("colossus.mobile.text-size", "large"),
  );
  await page.route("**/src/preview.ts*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body:
        (await response.text()) +
        `
activity.get("task-2").push({run_id:"run-2",sequence:5,created_at:new Date().toISOString(),update:{message:{content:[{artifact:{artifact_id:"safe-area-artifact",file_name:"safe-area-check.txt",media_type:"text/plain",size_bytes:40,state:"available"}}]}}});
`,
    });
  });
  try {
    await page.setViewportSize(portrait.viewport);
    await cdp.send("Emulation.setSafeAreaInsetsOverride", {
      insets: portrait.insets,
    });
    await page.goto("/");
    await page
      .getByRole("button", { name: "Explore development preview", exact: true })
      .click();
    await page.getByRole("button", { name: /Prepare release notes/ }).click();
    await page
      .getByRole("button", { name: "Session views", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Session view", exact: true })
      .click();
    await page.getByRole("option", { name: "Resources", exact: true }).click();
    for (const { viewport, insets } of [portrait, landscape, narrow]) {
      await page.setViewportSize(viewport);
      await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets });
      await page.getByRole("button", { name: /safe-area-check.txt/ }).click();
      await expect(
        page.getByRole("dialog", { name: "safe-area-check.txt", exact: true }),
      ).toBeVisible();
      const dismiss = page.locator(
        ".mobile-session-sheet .ui-sheet-close-button",
      );
      await unobscured(dismiss, viewport, insets);
      const detailsClose = page.getByRole("button", {
        name: "Close details",
        exact: true,
      });
      await detailsClose.scrollIntoViewIfNeeded();
      await unobscured(detailsClose, viewport, insets);
      await expect(page.getByText("40 bytes", { exact: true })).toBeVisible();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: `test-results/safe-area-artifact-${viewport.width}.png`,
      });
      await detailsClose.click();
      await expect(
        page.getByRole("dialog", { name: "safe-area-check.txt", exact: true }),
      ).toHaveCount(0);
    }
  } finally {
    await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: {} });
    await cdp.detach();
  }
});

test("Large Settings controls remain reachable by user scrolling in landscape and narrow layouts", async ({
  page,
  browserName,
}) => {
  test.skip(
    browserName !== "chromium",
    "Safe-area emulation uses Chromium CDP.",
  );
  const cdp = await safeAreaSession(page);
  await page.addInitScript(() =>
    localStorage.setItem("colossus.mobile.text-size", "large"),
  );
  try {
    await page.goto("/");
    await page
      .getByRole("button", { name: "Explore development preview", exact: true })
      .click();
    for (const { viewport, insets } of [landscape, narrow]) {
      await page.setViewportSize(viewport);
      await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets });
      await page.getByRole("button", { name: "Work", exact: true }).click();
      await page.getByRole("button", { name: "Settings", exact: true }).click();
      const settings = page.locator(".mobile-settings");
      const signOut = page.getByRole("button", {
        name: "Sign out",
        exact: true,
      });
      await expect(signOut).not.toBeInViewport();
      const bounds = (await settings.boundingBox())!;
      await page.mouse.move(
        bounds.x + bounds.width / 2,
        bounds.y + bounds.height / 2,
      );
      await page.mouse.wheel(0, 1600);
      await expect(signOut).toBeInViewport({ ratio: 1 });
      await unobscured(signOut, viewport, insets);
      const size = page.getByRole("combobox", {
        name: "Text size",
        exact: true,
      });
      await unobscured(size, viewport, insets);
      await size.click();
      await page.getByRole("option", { name: "Large", exact: true }).click();
      await expect(page.locator("html")).toHaveAttribute(
        "data-text-size",
        "large",
      );
      await page.screenshot({
        path: `test-results/safe-area-settings-${viewport.width}.png`,
      });
    }
  } finally {
    await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: {} });
    await cdp.detach();
  }
});
