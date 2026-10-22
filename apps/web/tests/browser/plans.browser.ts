import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";

for (const appearance of appearanceCases) {
  test(`personal Plan continuation reviews exact actions in a narrow Large ${appearance.label} layout`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, { plan: true });
    await selectAppearance(page, appearance);
    await page.goto("/projects/browser-project/chat/browser-thread");
    await page.getByRole("button", { name: "Read plan", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("Review connected sources.");
    await page.screenshot({
      path: `output/playwright/chat-plan-read-${appearance.id}-large.png`,
    });
    await dialog.getByText("Execute as bounded Goal", { exact: true }).click();
    await dialog
      .getByRole("button", { name: "Review Goal execution", exact: true })
      .click();
    const composer = page.getByRole("textbox", {
      name: "Message the agent",
      exact: true,
    });
    await expect(composer).toHaveValue(
      "Approve and run Plan revision 2 as a bounded Goal with 5 iterations.",
    );
    await expect(page.locator(".composer-plan-selection")).toContainText(
      "Plan revision 2 · Approve and execute",
    );
    const role = page.locator(".composer-fixed-role");
    await expect(role).toHaveText("Role: primary");
    await expect(role).toBeVisible();
    await expect(
      page.getByRole("combobox", { name: "Agent role", exact: true }),
    ).toHaveCount(0);
    expect(
      await role.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/chat-plan-approval-${appearance.id}-large.png`,
    });
    await page
      .getByRole("button", { name: "Approve and execute plan", exact: true })
      .click();
    await expect.poll(() => fixture.submissions.length).toBe(1);
    await expect(role).toHaveCount(0);
    await expect(
      page.getByRole("combobox", { name: "Agent role", exact: true }),
    ).toBeEnabled();
    expect(fixture.submissions[0]?.request).toMatchObject({
      session_id: null,
      mode: "execute",
      plan_action: {
        execute: {
          source_run_id: "browser-plan-run",
          expected_revision: 2,
          strategy: { goal: { max_iterations: 5 } },
        },
      },
    });
    await page.getByRole("button", { name: "Plans", exact: true }).click();
    await page.getByRole("button", { name: "Revise", exact: true }).click();
    await composer.fill("Add a review before the second step");
    await page
      .getByRole("button", { name: "Revise plan", exact: true })
      .click();
    await expect.poll(() => fixture.submissions.length).toBe(2);
    expect(fixture.submissions[1]?.request).toMatchObject({
      mode: "plan",
      plan_action: {
        revise: { source_run_id: "browser-plan-run", expected_revision: 2 },
      },
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/chat-plan-${appearance.id}-large.png`,
    });
    fixture.verify();
  });
}
