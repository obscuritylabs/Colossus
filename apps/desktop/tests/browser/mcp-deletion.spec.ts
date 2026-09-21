import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "MCP", exact: true }).click();
});

test("a newly added MCP server can be deleted immediately", async ({
  page,
}) => {
  await page
    .getByRole("button", { name: "Add MCP server", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Server name", exact: true })
    .fill("temporary-server");
  await page
    .getByRole("textbox", { name: "Executable", exact: true })
    .fill("temporary-mcp");
  await page.getByRole("button", { name: "Add server", exact: true }).click();
  const trigger = page.getByRole("button", {
    name: "Delete temporary-server",
    exact: true,
  });
  await trigger.click();
  await page
    .getByRole("dialog", { name: "Delete temporary-server?" })
    .getByRole("button", { name: "Delete server" })
    .click();
  await expect(trigger).toHaveCount(0);
  await expect(
    page.getByText("MCP server deleted.", { exact: true }),
  ).toBeVisible();
});

test("MCP deletion confirms, contains keyboard focus, and updates the list", async ({
  page,
}) => {
  await page.setViewportSize({ width: 880, height: 640 });
  const trigger = page.getByRole("button", {
    name: "Delete splunk-search",
    exact: true,
  });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Delete splunk-search?" });
  const cancel = dialog.getByRole("button", { name: "Cancel", exact: true });
  const confirm = dialog.getByRole("button", {
    name: "Delete server",
    exact: true,
  });
  await expect(cancel).toBeFocused();
  await expect(dialog).toContainText(
    "Saved credentials and OAuth data will be kept.",
  );
  await page.keyboard.press("Tab");
  await expect(confirm).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(cancel).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  // Native modal dialogs keep background controls inert even at the tab boundary.
  await expect(page.locator("#add-mcp-server")).not.toBeFocused();
  const accessibility = await new AxeBuilder({ page })
    .include(".mcp-delete-dialog")
    .analyze();
  expect(
    accessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await dialog.screenshot({
    path: "output/playwright/mcp-delete-confirmation.png",
  });
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await cancel.click();
  await expect(trigger).toBeFocused();
  await page
    .getByRole("button", { name: "Edit splunk-search", exact: true })
    .click();
  await trigger.click();
  await confirm.click();
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Edit MCP server", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("MCP server deleted.", { exact: true }),
  ).toBeVisible();
  await expect(page.locator("#add-mcp-server")).toBeFocused();
  await expect(
    page.getByRole("table", { name: "Global MCP servers" }).getByRole("row"),
  ).toHaveCount(3);
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await page.getByRole("button", { name: "MCP", exact: true }).click();
  await expect(
    page.getByRole("switch", { name: "Enable splunk-search", exact: true }),
  ).toHaveCount(0);
});

test("MCP deletion requires workspace disablement to be applied", async ({
  page,
}) => {
  const trigger = page.getByRole("button", {
    name: "Delete github-local",
    exact: true,
  });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: "Delete github-local?" });
  await expect(dialog.getByRole("listitem")).toHaveText(["Colossus"]);
  await expect(
    dialog.getByRole("button", { name: "Delete server" }),
  ).toBeDisabled();
  await dialog.screenshot({ path: "output/playwright/mcp-delete-blocked.png" });
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await page.getByRole("button", { name: "MCP", exact: true }).click();
  await page
    .getByRole("switch", { name: "Enable github-local", exact: true })
    .uncheck();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await trigger.click();
  await expect(
    dialog.getByRole("button", { name: "Delete server" }),
  ).toBeDisabled();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await page
    .getByRole("button", { name: "Apply Workspace changes", exact: true })
    .click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await trigger.click();
  await expect(
    dialog.getByRole("button", { name: "Delete server" }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "Delete server" }).click();
  await expect(trigger).toHaveCount(0);
  await page.getByRole("button", { name: "Credentials", exact: true }).click();
  await expect(
    page.getByText("GitHub workspace token", { exact: true }),
  ).toBeVisible();
});

test("MCP deletion stays busy during the native save and retains the row on failure", async ({
  page,
}) => {
  await page
    .getByRole("button", { name: "Delete splunk-search", exact: true })
    .click();
  await page.evaluate(() => {
    const state = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      mcpDeleteCalls: unknown[];
      rejectMcpDelete: (inUse?: boolean) => void;
    };
    state.mcpDeleteCalls = [];
    state.__TAURI_INTERNALS__ = {
      invoke: (command: string, args: unknown) => {
        state.mcpDeleteCalls.push({ command, args });
        return new Promise((_resolve, reject) => {
          state.rejectMcpDelete = (inUse = false) =>
            reject({
              code: inUse ? "invalid_argument" : "busy",
              retryable: !inUse,
              outcomeUnknown: false,
              violations: inUse
                ? [
                    {
                      field: "resourceId",
                      description:
                        "Disable this MCP server and apply changes in these Workspaces before deleting it: Engineering (archived). Restore archived Workspaces first.",
                    },
                  ]
                : [],
              message: inUse
                ? "The request is invalid."
                : "Global settings changed in another window. Reload and review the latest revision.",
            });
        });
      },
    };
  });
  const dialog = page.getByRole("dialog", { name: "Delete splunk-search?" });
  await dialog.getByRole("button", { name: "Delete server" }).click();
  await expect(dialog.getByRole("status")).toHaveText("Deleting MCP server…");
  await expect(
    dialog.getByRole("button", { name: "Delete server" }),
  ).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  const calls = await page.evaluate(
    () => (window as unknown as { mcpDeleteCalls: unknown[] }).mcpDeleteCalls,
  );
  expect(calls).toEqual([
    {
      command: "delete_global_mcp_server",
      args: {
        request: {
          expectedRevision: 4,
          resourceId: "018f1000-0000-7000-8000-000000000002",
        },
      },
    },
  ]);
  await page.evaluate(() =>
    (window as unknown as { rejectMcpDelete: () => void }).rejectMcpDelete(),
  );
  await expect(dialog.getByRole("alert")).toContainText(
    "Reload and review the latest revision.",
  );
  await expect(
    page.getByText("MCP server deleted.", { exact: true }),
  ).toHaveCount(0);
  await dialog.getByRole("button", { name: "Delete server" }).click();
  await page.evaluate(() =>
    (
      window as unknown as { rejectMcpDelete: (inUse: boolean) => void }
    ).rejectMcpDelete(true),
  );
  await expect(dialog.getByRole("alert")).toContainText(
    "Engineering (archived)",
  );
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(
    page.getByRole("button", { name: "Delete splunk-search", exact: true }),
  ).toBeVisible();
});
