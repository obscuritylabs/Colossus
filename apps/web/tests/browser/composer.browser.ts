import { expect, test, type Page } from "@playwright/test";
import { browserFixture } from "./fixture";

async function noHorizontalOverflow(page: Page) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
}
async function selectByKeyboard(page: Page, label: string, steps: number) {
  const trigger = page.getByRole("button", {
    name: /Choose model and reasoning:/,
  });
  await trigger.scrollIntoViewIfNeeded();
  await expect(trigger).toBeInViewport({ ratio: 1 });
  await trigger.press("Enter");
  const search = page.getByRole("combobox", {
    name: "Search models",
    exact: true,
  });
  await expect(search).toBeFocused();
  await search.fill(label);
  // The role-default detail can also contain the requested model's name. Move
  // to the explicit configured option, then verify the keyboard selection.
  await search.press("End");
  await expect(
    page.locator('.ui-model-option[data-active="true"] strong'),
  ).toHaveText(label);
  await search.press("Enter");
  const slider = page.getByRole("slider", {
    name: "Reasoning effort for next message",
    exact: true,
  });
  await expect(slider).toBeFocused();
  await slider.press("Home");
  for (let index = 0; index < steps; index++) await slider.press("ArrowRight");
  await expect(slider).toHaveAttribute("aria-valuenow", String(steps));
  await expect(page.locator(".ui-model-picker")).toBeInViewport({ ratio: 1 });
  await noHorizontalOverflow(page);
  return { trigger, slider };
}
async function closePicker(page: Page) {
  await page.keyboard.press("Escape");
  await expect(page.locator(".ui-model-picker")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /Choose model and reasoning:/ }),
  ).toBeFocused();
}

for (const theme of ["dark", "light"] as const) {
  test(`configured models work in the actual app with Large text and ${theme} narrow layout`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page);
    await page.goto("/settings?project=browser-project");
    await page.locator("#appearance-text-size").click();
    await page.getByRole("option", { name: "Large", exact: true }).click();
    await page.locator("#appearance-color-theme").click();
    await page
      .getByRole("option", {
        name: theme === "dark" ? "Dark" : "Light",
        exact: true,
      })
      .click();
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.goto(
      "/projects/browser-project/agents/browser-node/threads?compose=1",
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    const { slider } = await selectByKeyboard(page, "Careful browser model", 3);
    await expect(slider).toHaveAttribute("aria-valuetext", "low");
    await page.screenshot({
      path: `output/playwright/composer-picker-${theme}-large.png`,
    });
    await closePicker(page);
    await page
      .getByRole("textbox", { name: "Start a conversation", exact: true })
      .fill("Start with the careful model");
    await page
      .getByRole("textbox", { name: "Start a conversation", exact: true })
      .press("Enter");
    await expect.poll(() => fixture.submissions.length).toBe(1);
    expect(fixture.submissions[0]).toMatchObject({
      path: "/api/projects/browser-project/threads",
      node_id: "browser-node",
      request: { model_profile: "careful", reasoning_effort: "low" },
    });
    await expect(
      page.getByRole("heading", {
        name: "Browser model conversation",
        exact: true,
      }),
    ).toBeVisible();

    const input = page.getByRole("textbox", {
      name: "Message the agent",
      exact: true,
    });
    await selectByKeyboard(page, "Fast browser model", 4);
    await closePicker(page);
    await input.fill("Continue with a different configured model");
    await input.press("Enter");
    await expect.poll(() => fixture.submissions.length).toBe(2);
    expect(fixture.submissions[1]).toMatchObject({
      path: "/api/projects/browser-project/threads/browser-thread/messages",
      request: { model_profile: "fast", reasoning_effort: "medium" },
    });

    await selectByKeyboard(page, "Role default", 0);
    await closePicker(page);
    await input.fill("Use the runtime role defaults");
    await input.press("Enter");
    await expect.poll(() => fixture.submissions.length).toBe(3);
    expect(fixture.submissions[2]!.request).not.toHaveProperty("model_profile");
    expect(fixture.submissions[2]!.request).not.toHaveProperty(
      "reasoning_effort",
    );

    await selectByKeyboard(page, "Fast browser model", 4);
    await closePicker(page);
    await page
      .locator(".mode-switch label")
      .filter({ hasText: "Research" })
      .click();
    await expect(
      page.getByRole("button", { name: /Choose model and reasoning:/ }),
    ).toHaveCount(0);
    await expect(
      page.getByText("Host research model", { exact: true }),
    ).toBeVisible();
    await input.fill("Research using the configured research route");
    await input.press("Enter");
    await expect.poll(() => fixture.submissions.length).toBe(4);
    expect(fixture.submissions[3]!.request).toMatchObject({
      mode: "research",
      research_sources: ["repo"],
    });
    expect(fixture.submissions[3]!.request).not.toHaveProperty("model_profile");
    expect(fixture.submissions[3]!.request).not.toHaveProperty(
      "reasoning_effort",
    );
    await page
      .locator(".mode-switch label")
      .filter({ hasText: "Execute" })
      .click();
    await expect(
      page.getByRole("button", {
        name: /Choose model and reasoning: Fast browser model/,
      }),
    ).toContainText("medium");
    await noHorizontalOverflow(page);
    await page.screenshot({
      path: `output/playwright/composer-conversation-${theme}-large.png`,
    });
    fixture.verify();
  });
}
