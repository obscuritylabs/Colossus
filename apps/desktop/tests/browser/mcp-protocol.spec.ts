import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("remote MCP protocol selection survives revisions and remains accessible", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "MCP", exact: true }).click();
  await page
    .getByRole("button", { name: "Add MCP server", exact: true })
    .click();
  const editor = page.locator(".mcp-server-editor");
  await editor
    .getByRole("textbox", { name: "Server name", exact: true })
    .fill("modern-mcp");
  await editor
    .getByRole("combobox", { name: "Transport", exact: true })
    .click();
  await page
    .getByRole("option", { name: "Remote endpoint (HTTP)", exact: true })
    .click();
  await editor
    .getByLabel("Server URL", { exact: true })
    .fill("https://mcp.example.test/rpc");
  await editor.getByRole("textbox", { name: /^Allowed tools/ }).fill("probe");
  const protocol = editor.getByRole("combobox", {
    name: "Protocol version",
    exact: true,
  });
  await expect(protocol).toContainText("Automatic");
  await protocol.focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(protocol).toContainText("2026-07-28");
  await expect(
    editor.getByText("Allow stateless HTTP", { exact: true }),
  ).toBeHidden();
  await page.getByRole("button", { name: "Add server", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit modern-mcp", exact: true })
    .click();
  await expect(protocol).toContainText("2026-07-28");
  await protocol.click();
  await page
    .getByRole("option", { name: "2025-11-25 compatibility", exact: true })
    .click();
  await expect(editor.getByText(/Accept missing session IDs/)).toBeVisible();
  const stateless = editor.getByRole("switch", {
    name: /Allow stateless HTTP/,
  });
  await stateless.check();
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit modern-mcp", exact: true })
    .click();
  await expect(protocol).toContainText("2025-11-25");
  await expect(stateless).toBeChecked();
  await protocol.click();
  await page.getByRole("option", { name: /Automatic/ }).click();
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit modern-mcp", exact: true })
    .click();
  await expect(protocol).toContainText("Automatic");
  await expect(stateless).toBeChecked();
  await protocol.click();
  await page.getByRole("option", { name: "2026-07-28", exact: true }).click();
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit modern-mcp", exact: true })
    .click();
  await expect(protocol).toContainText("2026-07-28");
  await expect(stateless).toBeHidden();
  await protocol.click();
  await page
    .getByRole("option", { name: "2025-11-25 compatibility", exact: true })
    .click();
  await expect(stateless).toBeChecked();

  for (const appearance of [
    { theme: "light", palette: "colossus", width: 1280 },
    { theme: "dark", palette: "colossus", width: 1280 },
    { theme: "dark", palette: "neutral", width: 700 },
    { theme: "dark", palette: "hacker", width: 700 },
  ]) {
    await page.setViewportSize({ width: appearance.width, height: 900 });
    await page.evaluate(({ theme, palette }) => {
      document.documentElement.dataset.theme = theme;
      document.documentElement.dataset.palette = palette;
      document.documentElement.dataset.textSize = "large";
    }, appearance);
    await protocol.scrollIntoViewIfNeeded();
    await expect(protocol).toBeVisible();
    expect(
      await editor.evaluate(
        (element) => element.scrollWidth - element.clientWidth,
      ),
    ).toBeLessThanOrEqual(1);
    await editor.screenshot({
      path: testInfo.outputPath(
        `mcp-protocol-${appearance.theme}-${appearance.palette}.png`,
      ),
    });
  }
  const accessibility = await new AxeBuilder({ page })
    .include(".mcp-server-editor")
    .analyze();
  expect(
    accessibility.violations.filter((item) =>
      ["critical", "serious"].includes(item.impact ?? ""),
    ),
  ).toEqual([]);
});
