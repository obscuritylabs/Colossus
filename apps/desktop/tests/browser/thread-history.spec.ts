import { expect, test } from "@playwright/test";

test("visible history fills incrementally, pauses for collapse/search and retries the exact page", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=thread-history&historyCount=300&historyFailAt=6");
  const sidebar = page.locator(".work-sidebar");
  await expect(sidebar.getByRole("alert")).toContainText(
    "temporarily unavailable",
  );
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-last-cursor",
    "history:6",
  );
  const requests = Number(
    await page.locator("html").getAttribute("data-history-requests"),
  );
  await page.waitForTimeout(250);
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-requests",
    String(requests),
  );
  await sidebar.getByRole("button", { name: "Retry", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(sidebar.getByRole("alert")).toHaveCount(0);
  const cursors = JSON.parse(
    (await page.locator("html").getAttribute("data-history-cursor-log"))!,
  ) as string[];
  expect(cursors.filter((cursor) => cursor === "history:6")).toHaveLength(2);
  await expect(
    sidebar.getByRole("button", { name: /^History thread 00007 Execute/ }),
  ).toBeVisible();
  await expect
    .poll(async () =>
      Number(await page.locator("html").getAttribute("data-history-requests")),
    )
    .toBeLessThan(16);
  await sidebar.getByRole("button", { name: /Collapse .* threads/ }).click();
  await page.waitForTimeout(150);
  const collapsed = await page
    .locator("html")
    .getAttribute("data-history-requests");
  await page.waitForTimeout(200);
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-requests",
    collapsed!,
  );
  await sidebar
    .getByRole("button", { name: "Expand Colossus threads", exact: true })
    .click();
  await sidebar
    .getByRole("searchbox", { name: /Search threads/ })
    .fill("missing");
  await page.waitForTimeout(150);
  const searching = await page
    .locator("html")
    .getAttribute("data-history-requests");
  await page.waitForTimeout(200);
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-requests",
    searching!,
  );
});

test("ordinary downward scrolling crosses 200 and 512 without manual page clicks", async ({
  page,
}, testInfo) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1280, height: 900 });
  const started = Date.now();
  await page.goto("/?fixture=thread-history&historyCount=530");
  const sidebar = page.locator(".work-sidebar");
  const scroll = sidebar.locator(".space-tree-scroll");
  await expect(
    sidebar.getByRole("button", { name: /^History thread 00001 Execute/ }),
  ).toBeVisible();
  await expect
    .poll(async () =>
      Number(await page.locator("html").getAttribute("data-history-requests")),
    )
    .toBeGreaterThan(1);
  const firstViewport = {
    milliseconds: Date.now() - started,
    requests: Number(
      await page.locator("html").getAttribute("data-history-requests"),
    ),
    domRows: await sidebar.locator("[data-history-row]").count(),
  };
  expect(firstViewport.requests).toBeLessThan(16);
  await scroll.hover();
  await expect
    .poll(
      async () => {
        await page.mouse.wheel(0, 5000);
        return sidebar
          .getByRole("button", { name: /^History thread 00530 Execute/ })
          .count();
      },
      { timeout: 45_000, intervals: [100] },
    )
    .toBe(1);
  await expect(
    sidebar.getByRole("button", { name: /^History thread 00530 Execute/ }),
  ).toBeVisible();
  expect(await sidebar.locator("[data-history-row]").count()).toBeLessThan(100);
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-maximum-in-flight",
    "1",
  );
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-requests",
    "177",
  );
  await testInfo.attach("synthetic-history-paging.json", {
    contentType: "application/json",
    body: JSON.stringify({
      firstViewport,
      total: 530,
      pages: 177,
      maximumInFlight: 1,
      milliseconds: Date.now() - started,
    }),
  });
});

test("more than 4096 loaded summaries retain bounded DOM, scrolling and keyboard selection", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(
    "/?fixture=thread-history&historyLoaded=4505&historyCount=4511",
  );
  const sidebar = page.locator(".work-sidebar");
  const scroll = sidebar.locator(".space-tree-scroll");
  await expect(
    sidebar.getByRole("button", { name: /^History thread 00001 Execute/ }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-requests",
    "1",
  );
  expect(await sidebar.locator("[data-history-row]").count()).toBeLessThan(100);
  await scroll.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(
    sidebar.getByRole("button", { name: /^History thread 04511 Execute/ }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute(
    "data-history-requests",
    "3",
  );
  await expect(
    sidebar.getByText("All available threads loaded", { exact: true }),
  ).toBeVisible();
  expect(await sidebar.locator("[data-history-row]").count()).toBeLessThan(100);
  const last = sidebar.getByRole("button", {
    name: /^History thread 04511 Execute/,
  });
  await last.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("heading", { name: "History thread 04511", exact: true }),
  ).toBeVisible();
  await scroll.evaluate((element) => {
    element.scrollTop = 20_000;
  });
  await expect
    .poll(async () => sidebar.locator("[data-history-row]").count())
    .toBeLessThan(100);
  const rows = sidebar.locator("[data-history-row]");
  const firstId = await rows.first().getAttribute("data-history-row");
  const first = rows.first().getByRole("button").first();
  await first.focus();
  await page.keyboard.press("Shift+Tab");
  const focusedId = await page.evaluate(
    () =>
      document.activeElement?.closest<HTMLElement>("[data-history-row]")
        ?.dataset.historyRow,
  );
  expect(focusedId).not.toBe(firstId);
  expect(focusedId).toMatch(/^history-run-/);
});

test("large text history remains measured across palettes, compact drawer, resize and rename", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(
    "/?fixture=thread-history&historyLoaded=4505&historyCount=4505",
  );
  const sidebar = page.locator(".work-sidebar");
  const scroll = sidebar.locator(".space-tree-scroll");
  await expect(sidebar.locator("[data-history-row]").first()).toBeVisible();
  await page.evaluate(() => {
    document.documentElement.dataset.textSize = "large";
  });
  for (const [theme, palette] of [
    ["light", "colossus"],
    ["dark", "colossus"],
    ["dark", "neutral"],
    ["dark", "black"],
    ["dark", "hacker"],
    ["dark", "forced"],
  ]) {
    await page.evaluate(
      ({ theme, palette }) => {
        document.documentElement.dataset.theme = theme;
        document.documentElement.dataset.palette =
          palette === "forced" ? "colossus" : palette;
      },
      { theme: theme!, palette: palette! },
    );
    await scroll.evaluate((element) => {
      element.scrollTop = 40_000;
    });
    await page.emulateMedia({
      forcedColors: palette === "forced" ? "active" : "none",
    });
    await expect.poll(() => overlappingRows(sidebar)).toBe(0);
    expect(await sidebar.locator("[data-history-row]").count()).toBeLessThan(
      100,
    );
    await sidebar.screenshot({
      path: `output/playwright/history-${theme}-${palette}-large.png`,
    });
  }
  await page.setViewportSize({ width: 1280, height: 650 });
  await expect.poll(() => overlappingRows(sidebar)).toBe(0);
  const action = sidebar
    .locator("[data-history-row]")
    .nth(10)
    .getByRole("button", { name: /^Thread actions for/ });
  await action.scrollIntoViewIfNeeded();
  await action.press("ArrowDown");
  await page.getByRole("menuitem", { name: /^Rename / }).press("Enter");
  await expect(
    sidebar.getByRole("textbox", { name: "Thread name", exact: true }),
  ).toBeFocused();
  await sidebar
    .getByRole("textbox", { name: "Thread name", exact: true })
    .press("Escape");
  await page.emulateMedia({ forcedColors: "none" });
  await page.evaluate(() => {
    document.documentElement.dataset.palette = "colossus";
  });
  await page.setViewportSize({ width: 560, height: 780 });
  await page
    .getByRole("button", { name: "Open work navigation", exact: true })
    .click();
  await expect(sidebar).toBeVisible();
  await expect
    .poll(() =>
      sidebar.evaluate((element) => {
        const close = element
          .querySelector('[aria-label="Close navigation"]')!
          .getBoundingClientRect();
        const manage = element
          .querySelector('[aria-label="Manage Workspaces"]')!
          .getBoundingClientRect();
        return manage.right < close.left;
      }),
    )
    .toBe(true);
  await scroll.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(
    sidebar.getByRole("button", { name: /^History thread 04505 Execute/ }),
  ).toBeVisible();
  await sidebar.screenshot({
    path: "output/playwright/history-compact-large.png",
  });
});

async function overlappingRows(sidebar: import("@playwright/test").Locator) {
  return sidebar.locator("[data-history-row]").evaluateAll((rows) => {
    const bounds = rows.map((row) => row.getBoundingClientRect());
    return bounds.filter(
      (rect, index) => index > 0 && rect.top < bounds[index - 1]!.bottom - 1,
    ).length;
  });
}
