import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";

for (const appearance of appearanceCases) {
  test(`project creation has its own page in ${appearance.label}`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, { globalAdmin: true });
    await selectAppearance(page, appearance);
    await page.goto("/projects");
    await page.getByRole("link", { name: "New project", exact: true }).click();
    await expect(page).toHaveURL(/\/projects\/new$/u);
    await expect(
      page.getByRole("heading", { name: "Create project", level: 1 }),
    ).toBeVisible();
    await expect(
      page.getByRole("table", { name: "Projects", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("textbox", { name: "Name", exact: true })
      .fill("New acceptance project");
    await page
      .getByRole("textbox", { name: "Description", exact: true })
      .fill("Created on a dedicated page.");
    await expect(
      page.getByRole("button", { name: "Save project", exact: true }),
    ).toBeEnabled();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/project-create-${appearance.id}.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(page).toHaveURL(/\/projects$/u);
    await page.goBack();
    await expect(page).toHaveURL(/\/projects\/new$/u);
    await page.reload();
    await page
      .getByRole("textbox", { name: "Name", exact: true })
      .fill("New acceptance project");
    await page
      .getByRole("button", { name: "Save project", exact: true })
      .click();
    await expect(page).toHaveURL(/\/projects$/u);
    await expect(
      page.getByRole("link", { name: "New acceptance project", exact: true }),
    ).toBeVisible();
    fixture.verify();
  });
}
test("project creation requires administrator access even for a direct URL", async ({
  page,
}) => {
  const fixture = await browserFixture(page);
  await page.goto("/projects/new");
  await expect(page.getByRole("alert")).toContainText(
    "Administrator access is required",
  );
  await expect(
    page.getByRole("button", { name: "Save project", exact: true }),
  ).toHaveCount(0);
  fixture.verify();
});
