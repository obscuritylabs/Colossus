import { expect, test } from "@playwright/test";

test("MCP editor preserves mixed patterns and exact selectors when saved and reopened", async ({
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
    .fill("pattern-tools");
  await editor
    .getByRole("textbox", { name: "Executable", exact: true })
    .fill("example-mcp-server");
  const selectors = editor.getByRole("textbox", { name: /^Allowed tools/ });
  await selectors.fill("get_*\n*_search\necho");
  await expect(selectors).toHaveAccessibleDescription(
    /zero or more characters/,
  );
  await expect(
    editor.getByText(/Patterns also include future matching tools/),
  ).toBeVisible();
  await editor.screenshot({
    path: testInfo.outputPath("mcp-pattern-selectors.png"),
  });
  await page.getByRole("button", { name: "Add server", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit pattern-tools", exact: true })
    .click();
  await expect(selectors).toHaveValue("get_*\n*_search\necho");
});
