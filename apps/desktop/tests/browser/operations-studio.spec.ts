import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const FIXTURE = "/?fixture=operations-studio";

test.beforeEach(async ({ page }) => {
  await page.goto(FIXTURE);
  await expect(
    page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
  ).toBeVisible();
});

test("minimum layout and capability-driven controls remain accessible", async ({
  page,
}) => {
  const layout = await page.evaluate(() => ({
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(layout).toEqual({
    innerWidth: 880,
    innerHeight: 640,
    scrollWidth: 880,
  });

  await expect(
    page.getByRole("button", { name: "Open files panel" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Open artifacts panel, 3 artifacts/u }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Attach a file" })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("button", { name: "Choose workspace context" }),
  ).toHaveCount(0);

  const results = await new AxeBuilder({ page }).analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);
});

test("required response card lines up with the prompt composer", async ({
  page,
}) => {
  await page.goto("/?fixture=interaction-question");

  const interaction = page.locator(
    ".pending-interaction-dock .interaction-card",
  );
  const composer = page.locator(".work-composer");
  await expect(interaction).toBeVisible();
  await expect(composer).toBeVisible();

  const [interactionBox, composerBox] = await Promise.all([
    interaction.boundingBox(),
    composer.boundingBox(),
  ]);
  expect(interactionBox).not.toBeNull();
  expect(composerBox).not.toBeNull();
  expect(Math.abs(interactionBox!.x - composerBox!.x)).toBeLessThan(1);
  expect(
    Math.abs(
      interactionBox!.x +
        interactionBox!.width -
        (composerBox!.x + composerBox!.width),
    ),
  ).toBeLessThan(1);
});

test("right-side drawers trap focus, close with Escape, and restore focus", async ({
  page,
}) => {
  const filesTrigger = page.getByRole("button", { name: "Open files panel" });
  await filesTrigger.click();

  const filesDialog = page.getByRole("dialog", { name: "Workspace files" });
  await expect(filesDialog).toBeVisible();
  await expect(
    filesDialog.getByRole("button", { name: "Close files drawer" }),
  ).toBeFocused();

  await page.keyboard.press("Shift+Tab");
  await expect(filesDialog).toContainText("Read-only");
  await page.keyboard.press("Escape");
  await expect(filesDialog).toHaveCount(0);
  await expect(filesTrigger).toBeFocused();

  const artifactsTrigger = page.getByRole("button", {
    name: /Open artifacts panel, 3 artifacts/u,
  });
  await artifactsTrigger.click();
  const artifactsDialog = page.getByRole("dialog", {
    name: "Artifact preview",
  });
  await expect(artifactsDialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(artifactsDialog).toHaveCount(0);
  await expect(artifactsTrigger).toBeFocused();
});

test("work navigation and approvals are keyboard-operable", async ({
  page,
}) => {
  const navigationTrigger = page.getByRole("button", {
    name: "Open work navigation",
  });
  await navigationTrigger.click();

  const navigation = page.getByRole("dialog", { name: "Work navigation" });
  await expect(navigation).toBeVisible();
  await expect(
    navigation.getByRole("button", { name: "Close work navigation" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(navigation).toHaveCount(0);
  await expect(navigationTrigger).toBeFocused();

  await expect(
    page.getByRole("heading", {
      name: "Apply the hardened bootstrap changes",
    }),
  ).toBeVisible();
  const allow = page.getByRole("button", { name: "Allow once" });
  await allow.focus();
  await page.keyboard.press("Enter");
  await expect(allow).toHaveCount(0);
  await expect(page.getByLabel("Required response")).toHaveCount(0);
});

test("follow-up prompts remain in the same work conversation", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Work navigation" })
    .getByRole("button", { name: "New work" })
    .click();

  const prompt = page.getByRole("textbox", { name: "Prompt" });
  const opening = "Review the updater configuration";
  const followUp = "Now check the Windows preview path";
  await prompt.fill(opening);
  await prompt.press("Enter");
  await expect(page.locator(".message-user .message-body")).toContainText(
    opening,
  );

  await prompt.fill(followUp);
  await prompt.press("Enter");
  await expect(page.locator(".message-user .message-body")).toHaveCount(2);
  await expect(
    page.locator(".message-user .message-body").nth(0),
  ).toContainText(opening);
  await expect(
    page.locator(".message-user .message-body").nth(1),
  ).toContainText(followUp);
  await expect(page.locator(".message-assistant")).toHaveCount(2);
  await expect(page.getByRole("heading", { name: opening })).toBeVisible();

  await page.getByRole("button", { name: "Open work navigation" }).click();
  const workNavigation = page.getByRole("dialog", {
    name: "Work navigation",
  });
  await expect(
    workNavigation.getByRole("button", { name: new RegExp(opening, "u") }),
  ).toHaveCount(1);
  await expect(workNavigation.getByText(followUp, { exact: true })).toHaveCount(
    0,
  );
});

test("high-contrast mode preserves visible focus and controls", async ({
  page,
}) => {
  await page.emulateMedia({ forcedColors: "active" });
  const filesTrigger = page.getByRole("button", { name: "Open files panel" });
  await filesTrigger.focus();
  await expect(filesTrigger).toBeFocused();
  await expect(filesTrigger).toBeVisible();

  const focusStyle = await filesTrigger.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
    };
  });
  expect(focusStyle.outlineStyle).not.toBe("none");
  expect(focusStyle.outlineWidth).not.toBe("0px");
});
