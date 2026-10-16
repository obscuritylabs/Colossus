import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { initialView, type WorkView } from "../../src/model.js";

function fixture(): WorkView {
  return {
    ...initialView("model-picker-workspace"),
    connected: true,
    status: "Ready",
    capabilities: [
      { name: "agent_runs.model_selection", enabled: true, detail: "" },
      { name: "research.create", enabled: true, detail: "" },
    ],
    modelProfiles: [
      {
        profile: "fast",
        model: "Fast model",
        provider: "OpenAI",
        providerProfile: "first",
        reasoningEffort: "low",
        defaultRoles: ["primary"],
      },
      {
        profile: "deep",
        model: "Deep model",
        provider: "Anthropic",
        providerProfile: "second",
        reasoningEffort: "high",
        defaultRoles: [],
      },
    ],
  };
}
async function open(page: Page, view = fixture()) {
  await page.addInitScript(() => {
    const host = window as unknown as {
      acquireVsCodeApi: () => unknown;
      actions: unknown[];
      view: WorkView;
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      getState() {},
      setState() {},
      postMessage(message: {
        type: string;
        selection?: WorkView["modelSelection"];
      }) {
        host.actions.push(message);
        if (message.type === "setModelSelection") {
          host.view = { ...host.view, modelSelection: message.selection! };
          window.postMessage({ type: "state", view: host.view }, "*");
        }
      },
    });
  });
  await page.goto("/");
  await publish(page, view);
}
async function publish(page: Page, view: WorkView) {
  await page.evaluate((view) => {
    (window as unknown as { view: WorkView }).view = view;
    window.postMessage({ type: "state", view }, "*");
  }, view);
}
async function actions(page: Page) {
  return page.evaluate(
    () => (window as unknown as { actions: unknown[] }).actions,
  );
}

test("one shared bubble searches providers, focuses reasoning, snapshots sends and resets model defaults", async ({
  page,
}) => {
  await open(page);
  const prompt = page.getByRole("textbox", { name: "Task for Colossus" });
  await prompt.fill("Keep this conversation draft");
  const trigger = page.getByRole("button", {
    name: /Choose model and reasoning/,
  });
  await trigger.click();
  const search = page.getByRole("combobox", { name: "Search models" });
  await search.fill("Anthropic");
  await expect(page.getByRole("option")).toHaveCount(1);
  await search.press("Enter");
  const slider = page.getByRole("slider", {
    name: "Reasoning effort for next message",
  });
  await expect(slider).toBeFocused();
  await expect(slider).toHaveAttribute(
    "aria-valuetext",
    "Model default · high",
  );
  await slider.press("Home");
  for (let i = 0; i < 3; i++) await slider.press("ArrowRight");
  await expect(slider).toHaveAttribute("aria-valuetext", "low");
  await page.getByRole("option", { name: /Deep model/ }).click();
  await expect(slider).toHaveAttribute("aria-valuetext", "low");
  await slider.press("Escape");
  await expect(trigger).toBeFocused();
  await expect(prompt).toHaveValue("Keep this conversation draft");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({
      type: "send",
      text: "Keep this conversation draft",
      mode: "plan",
      modelSelection: { modelProfile: "deep", reasoningEffort: "low" },
    });
  await trigger.click();
  await page.getByRole("option", { name: /Fast model OpenAI/ }).click();
  await expect(slider).toHaveAttribute("aria-valuetext", "Model default · low");
  await page.getByRole("option", { name: /Role default/ }).click();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({
      type: "send",
      text: "Keep this conversation draft",
      mode: "plan",
    });
});

test("model selection follows authenticated capability and Research routing", async ({
  page,
}) => {
  const view = fixture();
  await open(page, { ...view, capabilities: [] });
  const trigger = page.getByRole("button", {
    name: /Choose model and reasoning/,
  });
  await expect(trigger).toHaveCount(0);
  await publish(page, view);
  await expect(trigger).toBeVisible();
  await page
    .locator(".mode-switch")
    .getByText("Research", { exact: true })
    .click();
  await expect(trigger).toHaveCount(0);
  await page
    .locator(".mode-switch")
    .getByText("Execute", { exact: true })
    .click();
  await expect(trigger).toBeVisible();
  await publish(page, { ...view, busy: true, sessionId: "" });
  await expect(trigger).toBeDisabled();
  await publish(page, { ...view, busy: true, sessionId: "existing-session" });
  await expect(trigger).toBeEnabled();
});

test("shared picker stays accessible at compact widths, Large text and every host palette", async ({
  page,
}) => {
  await open(page);
  for (const [palette, light] of [
    ["editor", false],
    ["colossus", false],
    ["black", false],
    ["hacker", false],
    ["editor", true],
  ] as const) {
    await page.setViewportSize({ width: 320, height: 850 });
    await page.evaluate(
      ({ palette, light }) => {
        document.body.className = light ? "vscode-light" : "vscode-dark";
        document.documentElement.dataset.textSize = "large";
        window.postMessage({ type: "palette", palette }, "*");
      },
      { palette, light },
    );
    await page
      .getByRole("button", { name: /Choose model and reasoning/ })
      .click();
    const search = page.getByRole("combobox", { name: "Search models" });
    await expect(search).toBeFocused();
    await search.fill("no matching provider");
    await expect(page.getByText("No matching models")).toBeVisible();
    await search.fill("");
    const bounds = await page
      .getByRole("dialog", { name: "Model and reasoning" })
      .boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(320);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.screenshot({
      path: `artifacts/review/model-picker-${palette}-${light ? "light" : "dark"}.png`,
    });
    await search.press("Escape");
  }
  await page.emulateMedia({ forcedColors: "active" });
  await page
    .getByRole("button", { name: /Choose model and reasoning/ })
    .click();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "artifacts/review/model-picker-forced-colors.png",
  });
});
