import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { initialView, type WorkView } from "../../src/model.js";

async function open(page: Page, view: WorkView) {
  await page.addInitScript(() => {
    const host = window as unknown as {
      acquireVsCodeApi: () => unknown;
      actions: unknown[];
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      postMessage: (message: unknown) => host.actions.push(message),
      getState: () => JSON.parse(sessionStorage.getItem("composer") ?? "null"),
      setState: (state: unknown) =>
        sessionStorage.setItem("composer", JSON.stringify(state)),
    });
  });
  await page.goto("/");
  await publish(page, view);
}

async function publish(page: Page, view: WorkView) {
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    view,
  );
}

function researchView(): WorkView {
  return {
    ...initialView("project"),
    connected: true,
    status: "Ready",
    capabilities: [{ name: "research.create", enabled: true, detail: "" }],
  };
}

test("Research sends selected MCP evidence, retains settings on reload, and requires a source", async ({
  page,
}) => {
  const view = researchView();
  await open(page, view);
  await page
    .locator(".mode-switch")
    .getByText("Research", { exact: true })
    .click();
  await expect(page.getByRole("radio", { name: "Standard" })).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "This Workspace" }),
  ).toBeChecked();
  await page.getByRole("radio", { name: "Deep" }).check();
  await page.getByRole("checkbox", { name: "This Workspace" }).uncheck();
  await page.getByRole("checkbox", { name: "MCP connections" }).check();
  await page.getByRole("checkbox", { name: "Web", exact: true }).check();
  await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .fill("Investigate the incident");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "send",
      text: "Investigate the incident",
      mode: "research",
      researchDepth: "deep",
      researchSources: ["mcp", "web"],
    });

  await page.reload();
  await publish(page, view);
  await expect(
    page.getByRole("radio", { name: "Research", exact: true }),
  ).toBeChecked();
  await expect(page.getByRole("radio", { name: "Deep" })).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "MCP connections" }),
  ).toBeChecked();
  await expect(
    page.getByRole("textbox", { name: "Task for Colossus" }),
  ).toHaveValue("Investigate the incident");
  await page.getByRole("checkbox", { name: "MCP connections" }).uncheck();
  await page.getByRole("checkbox", { name: "Web", exact: true }).uncheck();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();
  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "Select at least one evidence source" }),
  ).toBeVisible();
  await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .press("Control+Enter");
  expect(
    await page.evaluate(() =>
      (window as unknown as { actions: { type: string }[] }).actions.filter(
        (action) => action.type === "send",
      ),
    ),
  ).toEqual([]);
  await page.reload();
  await publish(page, view);
  await expect(
    page.getByRole("checkbox", { name: "This Workspace" }),
  ).not.toBeChecked();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();

  await page.getByRole("checkbox", { name: "MCP connections" }).check();
  await page.locator(".mode-switch").getByText("Plan", { exact: true }).click();
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "send",
      text: "Investigate the incident",
      mode: "plan",
    });
  await publish(page, {
    ...view,
    busy: true,
    watching: true,
    mode: "research",
  });
  await expect(
    page.getByRole("radio", { name: "Research", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: "MCP connections" }),
  ).toBeDisabled();
  await expect(page.getByRole("radio", { name: "Deep" })).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
});

test("Research follows the worker capability without falling back to Execute", async ({
  page,
}) => {
  const view = researchView();
  await open(page, { ...view, capabilities: [] });
  await expect(
    page.getByRole("radio", { name: "Research", exact: true }),
  ).toBeDisabled();
  await publish(page, view);
  await page
    .locator(".mode-switch")
    .getByText("Research", { exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .fill("Research question");
  await publish(page, {
    ...view,
    capabilities: [{ name: "research.create", enabled: false, detail: "" }],
  });
  await expect(
    page.getByRole("radio", { name: "Research", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByText("Research is unavailable for this worker connection.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .press("Control+Enter");
  expect(
    await page.evaluate(() =>
      (window as unknown as { actions: { type: string }[] }).actions.filter(
        (action) => action.type === "send",
      ),
    ),
  ).toEqual([]);
});

test("Research controls remain accessible across palettes, narrow layouts, and Large text", async ({
  page,
}) => {
  await open(page, researchView());
  await page
    .locator(".mode-switch")
    .getByText("Research", { exact: true })
    .click();
  await page.getByRole("checkbox", { name: "MCP connections" }).check();
  await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .fill("What do the connected sources say?");
  for (const [name, theme, palette, width, size] of [
    ["wide-light", "vscode-light", "editor", 640, "comfortable"],
    ["compact-dark", "vscode-dark", "editor", 280, "large"],
    ["narrow", "vscode-dark", "editor", 260, "large"],
    ["colossus", "vscode-dark", "colossus", 360, "comfortable"],
    ["hacker", "vscode-dark", "hacker", 360, "large"],
    ["high-contrast", "vscode-high-contrast", "editor", 280, "large"],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(
      ({ theme, palette, size }) => {
        document.body.className = theme;
        document.documentElement.dataset.textSize = size;
        window.postMessage({ type: "palette", palette }, "*");
      },
      { theme, palette, size },
    );
    await expect(
      page.getByRole("checkbox", { name: "MCP connections" }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    const send = await page
      .getByRole("button", { name: "Send", exact: true })
      .boundingBox();
    const modes = await page.locator(".mode-switch").boundingBox();
    expect(send).not.toBeNull();
    expect(modes).not.toBeNull();
    expect(
      send!.x >= modes!.x + modes!.width || send!.y + send!.height <= modes!.y,
    ).toBe(true);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({
      path: `artifacts/research-${name}.png`,
      fullPage: true,
    });
  }
  await page.getByRole("checkbox", { name: "MCP connections" }).focus();
  await page.keyboard.press("Space");
  await expect(
    page.getByRole("checkbox", { name: "MCP connections" }),
  ).not.toBeChecked();
});
