import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/?fixture=plan-workflow");
  await page
    .getByRole("button", { name: "Close details drawer", exact: true })
    .click();
});

test("revising a plan retires the prior draft's actions", async ({ page }) => {
  const cards = page.locator(".plan-result-card");
  await cards.getByRole("button", { name: "Revise in chat" }).click();
  const prompt = page.getByRole("textbox", { name: "Prompt" });
  await prompt.fill("Include rollback verification.");
  await prompt.press("Enter");
  await expect(cards).toHaveCount(2);
  await page
    .getByRole("button", { name: "Close details drawer", exact: true })
    .click();
  await expect(
    cards.first().getByRole("button", { name: "Run once" }),
  ).toHaveCount(0);
  await expect(
    cards.first().getByRole("button", { name: "Revise in chat" }),
  ).toHaveCount(0);
  await expect(
    cards.last().getByRole("button", { name: "Run once" }),
  ).toBeEnabled();
  await expect(cards.last()).toContainText("Revision 4");
});

for (const strategy of ["Run once", "Run as Goal"] as const) {
  for (const status of ["queued", "waiting"] as const) {
    test(`${strategy} disables continuation immediately when accepted as ${status}`, async ({
      page,
    }) => {
      await page.goto(`/?fixture=plan-workflow&planExecutionStatus=${status}`);
      await page
        .getByRole("button", { name: "Close details drawer", exact: true })
        .click();
      const cards = page.locator(".plan-result-card");
      await cards.getByRole("button", { name: strategy, exact: true }).click();
      await expect(
        page.getByRole("radio", { name: "Execute", exact: true }),
      ).toBeChecked();
      await expect(
        page.locator(".conversation-timeline > [data-aside-source-run-id]"),
      ).toHaveCount(2);
      await expect(cards).toHaveCount(1);
      await expect(cards.getByRole("button", { name: "Run once" })).toHaveCount(
        0,
      );
      await expect(
        cards.getByRole("button", { name: "Revise in chat" }),
      ).toHaveCount(0);
      await page.getByRole("button", { name: "Plans", exact: true }).click();
      const plan = page.locator(".session-plan-list article");
      await expect(
        plan.getByRole("button", { name: "Revise", exact: true }),
      ).toBeDisabled();
      await plan.getByRole("button", { name: "Read plan" }).click();
      await expect(
        page
          .locator(".plan-details-actions")
          .getByRole("button", { name: "Revise in chat" }),
      ).toBeDisabled();
    });
  }

  test(`${strategy} retires draft actions in the conversation and plan inspectors`, async ({
    page,
  }) => {
    const prompt = page.getByRole("textbox", { name: "Prompt" });
    await prompt.fill("/plan on");
    await prompt.press("Enter");
    await expect(
      page.getByRole("radio", { name: "Plan", exact: true }),
    ).toBeChecked();
    const cards = page.locator(".plan-result-card");
    await cards.getByRole("button", { name: strategy, exact: true }).click();
    await expect(cards).toHaveCount(2);
    await expect(
      page.getByRole("radio", { name: "Execute", exact: true }),
    ).toBeChecked();
    await expect(cards.getByRole("button", { name: "Run once" })).toHaveCount(
      0,
    );
    await expect(
      cards.getByRole("button", { name: "Revise in chat" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Close details drawer", exact: true }),
    ).toHaveCount(0);

    await page.getByRole("button", { name: "Plans", exact: true }).click();
    const plan = page.locator(".session-plan-list article");
    await expect(plan).toHaveCount(1);
    await expect(plan).toContainText("Executed");
    await expect(plan).toContainText("Desktop Plan workflow");
    await expect(
      plan.getByRole("button", { name: "Revise", exact: true }),
    ).toBeDisabled();
    await plan.getByRole("button", { name: "Read plan" }).click();
    await expect(
      page
        .locator(".plan-details-actions")
        .getByRole("button", { name: "Revise in chat" }),
    ).toBeDisabled();
  });
}
