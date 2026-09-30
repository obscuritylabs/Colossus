import { expect, test } from "@playwright/test";

test("tool switching preserves browser tabs and the conversation draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Keep this draft while switching tools.");
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
  const address = page.getByRole("textbox", { name: "Web address" });
  await address.fill("https://example.com/docs");
  await address.press("Enter");
  await page.getByRole("button", { name: "Switch pane tool" }).click();
  await page.getByRole("menuitemradio", { name: /^Terminal/ }).click();
  await expect(
    page.getByRole("region", { name: "Terminal pane" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Switch pane tool" }).click();
  await page.getByRole("menuitemradio", { name: /^Browser/ }).click();
  await expect(address).toHaveValue("https://example.com/docs");
  await expect(prompt).toHaveValue("Keep this draft while switching tools.");
});

test("compact terminal reserves native bounds and tool menus work with the keyboard", async ({
  page,
}) => {
  await page.setViewportSize({ width: 880, height: 640 });
  await page.goto("/?fixture=operations-studio");
  const tools = page.getByRole("button", { name: "Open tools", exact: true });
  await tools.focus();
  await tools.press("ArrowDown");
  await page.keyboard.press("Home");
  await expect(
    page.getByRole("menuitemradio", { name: /^Files/ }),
  ).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(
    page.getByRole("menuitemradio", { name: /^Terminal/ }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  const terminal = page.getByRole("region", { name: "Terminal pane" });
  await expect(terminal).toBeVisible();
  const bounds = await terminal.boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(48);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(641);
  const switcher = page.getByRole("button", { name: "Switch pane tool" });
  await switcher.click();
  await page.keyboard.press("Escape");
  await expect(switcher).toBeFocused();
  await expect(terminal).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(terminal).toHaveCount(0);
  await expect(tools).toBeFocused();
});
