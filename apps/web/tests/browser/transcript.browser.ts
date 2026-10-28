import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

const releasedMessages = () => [
  {
    message_id: "deployment-question",
    role: "user",
    text: "Check the deployment in 東京",
    created_at: "2026-10-16T10:01:00Z",
  },
  {
    message_id: "deployment-answer",
    role: "assistant",
    text: "Deployment is ready.\n\n```sh\nkubectl get pods\n```",
    created_at: "2026-10-16T10:02:00Z",
  },
  {
    message_id: "system-only",
    role: "system",
    text: "System-only diagnostic marker",
    created_at: "2026-10-16T10:03:00Z",
  },
];
const destination = "/projects/browser-project/chat/browser-thread";

test("loaded transcript search keeps selection during refresh and downloads only displayed chat text", async ({
  page,
}, testInfo) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    permissions: ["read"],
    releasedMessages: releasedMessages(),
  });
  await page.setViewportSize({ width: 1210, height: 954 });
  await page.goto(destination);
  const find = page.getByRole("button", {
    name: "Find in loaded conversation",
    exact: true,
  });
  await find.focus();
  await find.press("Enter");
  const input = page.getByRole("searchbox", {
    name: "Find loaded messages",
    exact: true,
  });
  await expect(input).toBeFocused();
  await input.fill("deployment");
  await expect(page.locator(".conversation-find-count")).toHaveText(
    "1 of 2 matching messages",
  );
  await expect(page.locator(".conversation-find-active")).toHaveAttribute(
    "data-aside-message-id",
    "deployment-question",
  );
  await input.press("Enter");
  const active = page.locator(".conversation-find-active");
  await expect(input).toBeFocused();
  await expect(active).toHaveAttribute(
    "data-aside-message-id",
    "deployment-answer",
  );
  await input.press("Enter");
  await expect(input).toBeFocused();
  await expect(active).toHaveAttribute(
    "data-aside-message-id",
    "deployment-question",
  );
  await input.press("Shift+Enter");
  await expect(input).toBeFocused();
  await expect(active).toHaveAttribute(
    "data-aside-message-id",
    "deployment-answer",
  );
  fixture.replaceReleasedMessage(
    "deployment-answer",
    releasedMessages()[1]!.text + "\nUpdated released answer.",
  );
  await expect(active).toContainText("Updated released answer.");
  await expect(active).toHaveAttribute(
    "data-aside-message-id",
    "deployment-answer",
  );
  await expect(input).toHaveValue("deployment");
  await input.press("Escape");
  await expect(input).toBeHidden();
  await expect(find).toBeFocused();
  const menu = page.getByRole("button", {
    name: "Conversation actions",
    exact: true,
  });
  await menu.focus();
  await menu.press("Enter");
  const downloadAction = page.getByRole("menuitem", {
    name: "Download loaded conversation",
    exact: true,
  });
  await downloadAction.focus();
  const downloadPromise = page.waitForEvent("download");
  await downloadAction.press("Enter");
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe(
    "Browser model conversation-loaded.md",
  );
  const file = testInfo.outputPath("loaded-conversation.md");
  await download.saveAs(file);
  const markdown = await readFile(file, "utf8");
  expect(markdown).toContain(
    "only loaded, released user and assistant messages",
  );
  expect(markdown).toContain("Check the deployment in 東京");
  expect(markdown).toContain("```sh\nkubectl get pods\n```");
  expect(markdown).toContain("Updated released answer.");
  expect(markdown).not.toContain("System-only diagnostic marker");
  expect(markdown).not.toContain("browser-thread");
  await expect(menu).toBeFocused();
  await page.screenshot({
    path: testInfo.outputPath("transcript-desktop.png"),
    fullPage: true,
  });
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`transcript actions fit narrow Large text in ${appearance.label}`, async ({
    page,
  }, testInfo) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      permissions: ["read"],
      releasedMessages: releasedMessages(),
    });
    await selectAppearance(page, appearance);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(destination);
    const find = page.getByRole("button", {
      name: "Find in loaded conversation",
      exact: true,
    });
    await find.focus();
    await find.press("Enter");
    const input = page.getByRole("searchbox", {
      name: "Find loaded messages",
      exact: true,
    });
    await expect(input).toBeFocused();
    await input.fill("deployment");
    const search = page.getByRole("search", {
      name: "Find in loaded conversation",
      exact: true,
    });
    await expect(search).toBeVisible();
    await expect(page.locator(".conversation-find-count")).toHaveText(
      "1 of 2 matching messages",
    );
    const dimensions = await search.evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      return {
        right: bounds.right,
        width: bounds.width,
        client: element.clientWidth,
        scroll: element.scrollWidth,
        controls: [...element.querySelectorAll("button, input")].map(
          (control) => {
            const rect = control.getBoundingClientRect();
            return { left: rect.left, right: rect.right, height: rect.height };
          },
        ),
      };
    });
    expect(dimensions.right).toBeLessThanOrEqual(391);
    expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.client + 1);
    for (const control of dimensions.controls) {
      expect(control.left).toBeGreaterThanOrEqual(0);
      expect(control.right).toBeLessThanOrEqual(391);
    }
    await page.screenshot({
      path: testInfo.outputPath(`transcript-find-${appearance.id}-large.png`),
      fullPage: true,
    });
    await input.press("Shift+Enter");
    await expect(page.locator(".conversation-find-active")).toHaveAttribute(
      "data-aside-message-id",
      "deployment-answer",
    );
    await page.locator(".conversation-find-active").press("Escape");
    await expect(find).toBeFocused();
    await page
      .getByRole("button", { name: "Conversation actions", exact: true })
      .press("Enter");
    const copy = page.getByRole("menuitem", {
      name: "Copy loaded conversation",
      exact: true,
    });
    await expect(copy).toBeVisible();
    await expect(copy).toBeEnabled();
    await expect(
      page.getByRole("menuitem", {
        name: "Download loaded conversation",
        exact: true,
      }),
    ).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath(`transcript-menu-${appearance.id}-large.png`),
      fullPage: true,
    });
    await copy.press("Escape");
    await expect(
      page.getByRole("button", { name: "Conversation actions", exact: true }),
    ).toBeFocused();
    fixture.verify();
  });
}
