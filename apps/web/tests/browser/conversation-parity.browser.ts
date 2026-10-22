import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  appearanceCases,
  openConversationList,
  selectAppearance,
} from "./appearance";
import { browserFixture } from "./fixture";

const parentUrl = "/projects/browser-project/chat/browser-thread";
const parentTitle = "Browser model conversation";

function mainComposer(page: Page) {
  return page
    .locator(".shared-inspector-main > .thread-detail")
    .getByRole("textbox", {
      name: "Message the agent",
      exact: true,
      includeHidden: true,
    });
}

async function expectTabVisible(tab: Locator) {
  // WebKit may round a focused tab's fractional scroll offset by a subpixel.
  await expect(tab).toBeInViewport({ ratio: 0.99 });
  await expect
    .poll(() =>
      tab.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const list = element
          .closest(".shared-workspace-pane-tabs")!
          .getBoundingClientRect();
        return Math.max(
          0,
          list.left - bounds.left,
          bounds.right - list.right,
          list.top - bounds.top,
          bounds.bottom - list.bottom,
        );
      }),
    )
    .toBeLessThanOrEqual(1);
}

async function selectReleasedQuote(page: Page, end: number) {
  await page
    .locator(
      '[data-aside-message-id="released-answer"] [data-aside-selectable="true"]',
    )
    .evaluate((element, length) => {
      const range = document.createRange();
      const text = element.querySelector("p")!.firstChild!;
      range.setStart(text, 0);
      range.setEnd(text, length);
      const selection = window.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
      document.dispatchEvent(new Event("selectionchange"));
    }, end);
}

for (const appearance of appearanceCases) {
  test(`wide plan and artifact inspectors preserve the ${appearance.label} Large conversation and composer`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      plan: true,
      branches: true,
    });
    await page.setViewportSize({ width: 1510, height: 1000 });
    await selectAppearance(page, appearance);
    await page.goto(parentUrl);
    const composer = mainComposer(page);
    await composer.fill("Keep the main conversation draft");
    const readPlan = page.getByRole("button", {
      name: "Read plan",
      exact: true,
    });
    await readPlan.focus();
    await readPlan.press("Enter");
    const inspector = page.getByRole("complementary", {
      name: "Plan",
      exact: true,
    });
    await expect(inspector).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(
      page.getByRole("separator", { name: "Resize inspector" }),
    ).toBeVisible();
    await expect(inspector).toContainText("Review connected sources.");
    await composer.fill("Continue typing beside the plan");
    await expect(composer).toHaveValue("Continue typing beside the plan");
    await page.screenshot({
      path: `output/playwright/conversation-plan-wide-${appearance.id}-large.png`,
    });
    if (appearance.id === "dark") {
      await page.setViewportSize({ width: 966, height: 1000 });
      await expect(inspector).toBeVisible();
      await expect
        .poll(
          async () =>
            (await page.locator(".shared-inspector-main").boundingBox())!.width,
        )
        .toBeLessThanOrEqual(450);
      const geometry = await page
        .locator(".shared-inspector-main")
        .evaluate((element) => {
          const header = element.querySelector<HTMLElement>(
            ".shared-work-header",
          )!;
          return {
            width: element.getBoundingClientRect().width,
            headerWidth: header.clientWidth,
            headerScrollWidth: header.scrollWidth,
            metadata: [
              ...header.querySelectorAll<HTMLElement>(
                ".surface-run-meta > span",
              ),
            ].map((span) => ({
              text: span.textContent,
              width: span.clientWidth,
              scrollWidth: span.scrollWidth,
            })),
          };
        });
      expect(geometry.width).toBeGreaterThanOrEqual(390);
      expect(geometry.width).toBeLessThanOrEqual(450);
      expect(geometry.headerScrollWidth).toBeLessThanOrEqual(
        geometry.headerWidth,
      );
      expect(
        geometry.metadata.every((span) => span.scrollWidth <= span.width),
      ).toBe(true);
      await test.info().attach("compact-inspector-layout.json", {
        body: JSON.stringify(geometry, null, 2),
        contentType: "application/json",
      });
      await page.screenshot({
        path: "output/playwright/conversation-plan-compact-main-large.png",
      });
      await page.setViewportSize({ width: 1510, height: 1000 });
    }
    const close = inspector.getByRole("button", {
      name: "Close plan",
      exact: true,
    });
    await close.focus();
    await close.press("Escape");
    await expect(inspector).toHaveCount(0);
    await expect(readPlan).toBeFocused();
    await expect(composer).toHaveValue("Continue typing beside the plan");
    await page.getByRole("button", { name: "Resources", exact: true }).click();
    await page.getByRole("button", { name: /evidence\.json/ }).click();
    const artifact = page.getByRole("complementary", {
      name: "Artifact",
      exact: true,
    });
    await expect(artifact).toContainText("application/json");
    await expect(artifact).toContainText("44 bytes");
    await expect(artifact.getByRole("link")).toHaveCount(0);
    await expect(composer).toHaveValue("Continue typing beside the plan");
    await page.screenshot({
      path: `output/playwright/conversation-artifact-wide-${appearance.id}-large.png`,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    fixture.verify();
  });
}

test("selection starts a side-chat draft and keeps its send, history, and composer independent", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    plan: true,
    branches: true,
  });
  await page.setViewportSize({ width: 1510, height: 1000 });
  await selectAppearance(page, appearanceCases[0]);
  await page.goto(parentUrl);
  const composer = mainComposer(page);
  await composer.fill("Parent draft stays here");
  await selectReleasedQuote(page, "Released evidence".length);
  const launcher = page.locator(".shared-conversation-selection-launcher");
  await expect(launcher).toBeVisible();
  // A normal server poll must not clear the user's selection or its launcher.
  await page.waitForResponse(
    (response) =>
      response.url().endsWith("/threads/browser-thread") &&
      response.request().method() === "GET",
  );
  await expect(launcher).toBeVisible();
  await launcher.click();
  const side = page.getByRole("complementary", {
    name: "Side chats",
    exact: true,
  });
  await expect(
    side.getByRole("region", { name: "New side chat", exact: true }),
  ).toBeVisible();
  await expect(side.locator("blockquote")).toHaveText("Released evidence");
  expect(fixture.branchSubmissions).toHaveLength(0);
  const question = side.getByRole("textbox", {
    name: "Side-chat question",
    exact: true,
  });
  await question.fill("Explain the evidence");
  await expect(composer).toHaveValue("Parent draft stays here");
  await page.screenshot({
    path: "output/playwright/conversation-aside-draft-wide.png",
  });
  await side
    .getByRole("button", { name: "Start side chat", exact: true })
    .click();
  await expect.poll(() => fixture.branchSubmissions.length).toBe(1);
  expect(fixture.branchSubmissions[0]).toMatchObject({
    kind: "aside",
    source_task_id: "browser-plan-task",
    expected_revision: 1,
    request: {
      session_id: null,
      plan_action: null,
      input: [
        { text: "Selected text (quoted context):\nReleased evidence" },
        { text: "Explain the evidence" },
      ],
    },
  });
  await expect(
    side.getByText("Independent branch answer", { exact: true }),
  ).toBeVisible();
  await expect(page).toHaveURL(parentUrl);
  await expect(composer).toHaveValue("Parent draft stays here");
  const childComposer = side.getByRole("textbox", {
    name: "Message side chat",
    exact: true,
  });
  await side
    .getByRole("group", { name: "Run mode", exact: true })
    .getByText("Plan", { exact: true })
    .click();
  await expect(
    side.getByRole("radio", { name: "Plan", exact: true }),
  ).toBeChecked();
  await expect(
    page
      .locator(".shared-inspector-main")
      .getByRole("radio", { name: "Execute", exact: true }),
  ).toBeChecked();
  const modeIds = await page
    .locator(".run-composer .mode-switch input")
    .evaluateAll((inputs) => inputs.map((input) => input.id));
  expect(modeIds.length).toBeGreaterThanOrEqual(8);
  expect(new Set(modeIds).size).toBe(modeIds.length);
  await childComposer.fill("Follow up only in the side chat");
  await side.getByRole("button", { name: "Send message", exact: true }).click();
  await expect.poll(() => fixture.submissions.length).toBe(1);
  expect(fixture.submissions[0]?.path).toContain(
    "/threads/browser-aside-1/messages",
  );
  expect(fixture.submissions[0]?.request.session_id).toBeNull();
  await expect(composer).toHaveValue("Parent draft stays here");
  await page.setViewportSize({ width: 390, height: 844 });
  const sheet = page.getByRole("dialog", { name: "Side chats", exact: true });
  await expect(sheet).toBeVisible();
  const childActions = sheet.getByRole("button", {
    name: "Conversation actions",
    exact: true,
  });
  await childActions.focus();
  await childActions.press("Enter");
  await expect(
    page.getByRole("menuitem", { name: "Archive", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(childActions).toBeFocused();
  await expect(sheet).toBeVisible();
  await childActions.click();
  await page.getByRole("menuitem", { name: "Archive", exact: true }).click();
  await expect.poll(() => fixture.threadMutations.length).toBe(1);
  await childActions.click();
  await page.getByRole("menuitem", { name: "Restore", exact: true }).click();
  await expect.poll(() => fixture.threadMutations.length).toBe(2);
  await expect(
    sheet.getByRole("textbox", { name: "Message side chat", exact: true }),
  ).toBeEnabled();
  const childPicker = sheet.getByRole("button", {
    name: /Choose model and reasoning:/,
  });
  await childPicker.focus();
  await childPicker.press("Enter");
  const modelSearch = page.getByRole("combobox", {
    name: "Search models",
    exact: true,
  });
  await expect(modelSearch).toBeFocused();
  await modelSearch.fill("Fast browser model");
  await modelSearch.press("End");
  await modelSearch.press("Enter");
  const reasoning = page.getByRole("slider", {
    name: "Reasoning effort for next message",
    exact: true,
  });
  await expect(reasoning).toBeFocused();
  await reasoning.press("Home");
  for (let index = 0; index < 4; index++) await reasoning.press("ArrowRight");
  await expect(reasoning).toHaveAttribute("aria-valuetext", "medium");
  await expect(
    page.locator(".shared-inspector-main .ui-model-trigger"),
  ).toHaveAttribute(
    "aria-label",
    "Choose model and reasoning: Careful browser model",
  );
  await expect(page.locator(".ui-model-picker")).toBeInViewport({ ratio: 1 });
  await page.screenshot({
    path: "output/playwright/conversation-aside-model-picker-narrow.png",
  });
  await page.keyboard.press("Escape");
  await expect(page.locator(".ui-model-picker")).toHaveCount(0);
  await expect(sheet).toBeVisible();
  await expect(childPicker).toBeFocused();
  await childPicker.click();
  await expect(modelSearch).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator(".ui-model-picker")).toHaveCount(0);
  await expect(sheet).toBeVisible();
  await expect(childPicker).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
  await page.setViewportSize({ width: 1510, height: 1000 });
  await page
    .getByRole("button", { name: "Open side chats", exact: true })
    .click();
  await expect(side).toBeVisible();
  const sideTitle = fixture.branchSubmissions[0]!.title;
  await expect(
    page
      .getByRole("navigation", { name: "Your chat history", exact: true })
      .getByRole("link", { name: sideTitle, exact: true }),
  ).toHaveCount(0);
  await side
    .getByRole("button", { name: "Close side chats", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Open side chats", exact: true })
    .click();
  await expect(
    side
      .getByRole("navigation", { name: "Open side chats", exact: true })
      .getByRole("button", { name: sideTitle, exact: true }),
  ).toBeVisible();
  await side
    .getByRole("button", { name: "Show archived", exact: true })
    .click();
  await expect(
    side.getByRole("navigation", { name: "Archived side chats", exact: true }),
  ).toHaveCount(1);
  await expect(
    side.getByText("No archived side chats.", { exact: true }),
  ).toBeVisible();
  await side.getByRole("button", { name: "Show open", exact: true }).click();
  const existingSideChat = side
    .getByRole("navigation", { name: "Open side chats", exact: true })
    .getByRole("button", { name: sideTitle, exact: true });
  await expect(existingSideChat).toBeVisible();
  const parentAside = page
    .locator('[data-aside-message-id="released-answer"]')
    .getByRole("button", { name: "Ask in side chat", exact: true });
  await parentAside.focus();
  await parentAside.press("Enter");
  const pendingQuestion = side.getByRole("textbox", {
    name: "Side-chat question",
    exact: true,
  });
  await pendingQuestion.fill(
    "Keep this unsent side question while I review history",
  );
  await expect(pendingQuestion).toBeFocused();
  await Promise.all([
    page.waitForResponse(
      (response) =>
        response.request().method() === "GET" &&
        response.url().endsWith("/threads/browser-thread"),
    ),
    page.waitForResponse(
      (response) =>
        response.request().method() === "GET" &&
        response.url().includes("/threads/browser-thread/branches?"),
    ),
  ]);
  await expect(pendingQuestion).toBeFocused();
  await existingSideChat.click();
  await expect(
    side.getByRole("heading", { name: sideTitle, exact: true }),
  ).toBeVisible();
  await expect(
    side.getByText("Independent branch answer", { exact: true }),
  ).toBeVisible();
  await side
    .getByRole("button", { name: "Back to side-chat draft", exact: true })
    .click();
  await expect(pendingQuestion).toHaveValue(
    "Keep this unsent side question while I review history",
  );
  await expect(side.locator("blockquote")).toHaveText(
    "Released evidence supports a careful review before proceeding.",
  );
  await expect(composer).toHaveValue("Parent draft stays here");
  expect(fixture.branchSubmissions).toHaveLength(1);
  await page.screenshot({
    path: "output/playwright/conversation-aside-wide.png",
  });
  fixture.verify();
});

test("a focused selection launcher returns focus on Escape and after inspector dismissal", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    plan: true,
    branches: true,
  });
  await page.setViewportSize({ width: 1510, height: 1000 });
  await page.goto(parentUrl);
  const composer = mainComposer(page);
  await composer.fill("Keyboard selection draft");
  await selectReleasedQuote(page, "Released".length);
  const launcher = page.locator(".shared-conversation-selection-launcher");
  await expect(launcher).toBeVisible();
  await launcher.focus();
  await launcher.press("Escape");
  await expect(launcher).toHaveCount(0);
  await expect(composer).toBeFocused();
  await selectReleasedQuote(page, "Released evidence".length);
  await expect(launcher).toBeVisible();
  await launcher.focus();
  await launcher.press("Enter");
  const side = page.getByRole("complementary", {
    name: "Side chats",
    exact: true,
  });
  await expect(side.locator("blockquote")).toHaveText("Released evidence");
  const close = side.getByRole("button", {
    name: "Close side chats",
    exact: true,
  });
  await close.focus();
  await close.press("Escape");
  await expect(side).toHaveCount(0);
  await expect(
    page.locator('[data-aside-message-id="released-answer"]'),
  ).toBeFocused();
  await expect(composer).toHaveValue("Keyboard selection draft");
  expect(fixture.branchSubmissions).toHaveLength(0);
  fixture.verify();
});

test("narrow keyboard side-chat inspector traps focus, restores the message action, and preserves the parent draft", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    plan: true,
    branches: true,
  });
  await selectAppearance(page, appearanceCases[0]);
  await page.goto(parentUrl);
  const composer = mainComposer(page);
  await composer.fill("Retain the narrow parent draft");
  const action = page
    .locator('[data-aside-message-id="released-answer"]')
    .getByRole("button", { name: "Ask in side chat", exact: true });
  await action.focus();
  await action.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Side chats", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator("blockquote")).toHaveText(
    "Released evidence supports a careful review before proceeding.",
  );
  const input = dialog.getByRole("textbox", {
    name: "Side-chat question",
    exact: true,
  });
  await input.fill("Draft only");
  await page.keyboard.press("Tab");
  expect(
    await dialog.evaluate((element) =>
      element.contains(document.activeElement),
    ),
  ).toBe(true);
  await page.screenshot({
    path: "output/playwright/conversation-aside-narrow-large.png",
  });
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(action).toBeFocused();
  await expect(composer).toHaveValue("Retain the narrow parent draft");
  expect(fixture.branchSubmissions).toHaveLength(0);
  await page
    .getByRole("button", { name: "Open side chats", exact: true })
    .click();
  await expect(
    dialog.getByRole("textbox", { name: "Side-chat question", exact: true }),
  ).toHaveValue("Draft only");
  await page.setViewportSize({ width: 1510, height: 1000 });
  const wide = page.getByRole("complementary", {
    name: "Side chats",
    exact: true,
  });
  await expect(wide).toBeVisible();
  await expect(
    wide.getByRole("textbox", { name: "Side-chat question", exact: true }),
  ).toHaveValue("Draft only");
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("textbox", { name: "Side-chat question", exact: true }),
  ).toHaveValue("Draft only");
  await page.keyboard.press("Escape");
  await expect(composer).toHaveValue("Retain the narrow parent draft");
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  fixture.verify();
});

test("workspace tabs retain the parent and side-chat drafts through keyboard switching, tab close, and pane dismissal", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    plan: true,
    branches: true,
  });
  await page.setViewportSize({ width: 1760, height: 1040 });
  await page.goto(parentUrl);
  const composer = mainComposer(page);
  await composer.fill("Retained parent workspace draft");
  await page
    .locator('[data-aside-message-id="released-answer"]')
    .getByRole("button", { name: "Ask in side chat", exact: true })
    .click();
  const question = page.getByRole("textbox", {
    name: "Side-chat question",
    exact: true,
  });
  await question.fill("Retained side workspace draft");
  await page.getByRole("button", { name: "Read plan", exact: true }).click();
  const tabs = page.getByRole("tablist", {
    name: "Workspace tabs",
    exact: true,
  });
  const planTab = tabs.getByRole("tab", {
    name: "Controlled browser plan",
    exact: true,
  });
  const sideTab = tabs.getByRole("tab", { name: "Side chats", exact: true });
  await expect(planTab).toHaveAttribute("aria-selected", "true");
  await expect(sideTab).toHaveAttribute("aria-selected", "false");
  await page.getByRole("button", { name: "Resources", exact: true }).click();
  await page.getByRole("button", { name: /evidence\.json/ }).click();
  const artifactTab = tabs.getByRole("tab", {
    name: "evidence.json",
    exact: true,
  });
  await expect(tabs.getByRole("tab")).toHaveCount(3);
  await expect(artifactTab).toHaveAttribute("aria-selected", "true");
  await artifactTab.focus();
  await artifactTab.press("Home");
  await expect(sideTab).toBeFocused();
  await expect(sideTab).toHaveAttribute("aria-selected", "true");
  await expectTabVisible(sideTab);
  await expect(question).toHaveValue("Retained side workspace draft");
  await expect(page.locator(".thread-aside-panel blockquote")).toHaveText(
    "Released evidence supports a careful review before proceeding.",
  );
  await sideTab.press("ArrowRight");
  await expect(planTab).toBeFocused();
  await expect(planTab).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("complementary", { name: "Plan", exact: true }),
  ).toContainText("Review connected sources.");
  await planTab.press("End");
  await expect(artifactTab).toBeFocused();
  await expect(artifactTab).toHaveAttribute("aria-selected", "true");
  await expectTabVisible(artifactTab);
  await expect(
    page.getByRole("complementary", { name: "Artifact", exact: true }),
  ).toContainText("application/json");
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(
    page.getByRole("dialog", { name: "Artifact", exact: true }),
  ).toBeVisible();
  await artifactTab.focus();
  await artifactTab.press("Home");
  await expect(sideTab).toBeFocused();
  await expectTabVisible(sideTab);
  await expect(question).toHaveValue("Retained side workspace draft");
  await sideTab.press("End");
  await expect(artifactTab).toBeFocused();
  await expectTabVisible(artifactTab);
  await page
    .getByRole("dialog", { name: "Artifact", exact: true })
    .getByRole("button", { name: "Expand right pane", exact: true })
    .click();
  await page.setViewportSize({ width: 1760, height: 1040 });
  await expect(
    page.getByRole("complementary", { name: "Artifact", exact: true }),
  ).toBeVisible();
  await expect(artifactTab).toBeFocused();
  await expect(composer).toHaveValue("Retained parent workspace draft");
  await page
    .getByRole("button", { name: "Restore right pane", exact: true })
    .click();
  await expectTabVisible(artifactTab);
  const separator = page.getByRole("separator", {
    name: "Resize inspector",
    exact: true,
  });
  await separator.focus();
  await separator.press("ArrowRight");
  await separator.press("ArrowRight");
  await expectTabVisible(artifactTab);
  await page
    .getByRole("button", { name: "Expand right pane", exact: true })
    .click();
  await artifactTab.focus();
  await artifactTab.press("Home");
  await expect(sideTab).toHaveAttribute("aria-selected", "true");
  await expect(question).toHaveValue("Retained side workspace draft");
  await expect(composer).toHaveValue("Retained parent workspace draft");
  await sideTab.press("End");
  await expect(artifactTab).toHaveAttribute("aria-selected", "true");
  await page
    .getByRole("button", { name: "Restore right pane", exact: true })
    .click();
  await expectTabVisible(artifactTab);
  await artifactTab.focus();
  await artifactTab.press("ArrowLeft");
  await expect(planTab).toBeFocused();
  await page
    .getByRole("button", {
      name: "Close Controlled browser plan tab",
      exact: true,
    })
    .click();
  await expect(planTab).toHaveCount(0);
  await expect(tabs.getByRole("tab")).toHaveCount(2);
  await expect(tabs.getByRole("tab", { selected: true })).toHaveCount(1);
  await sideTab.click();
  await expect(question).toHaveValue("Retained side workspace draft");
  await page
    .getByRole("complementary", { name: "Side chats", exact: true })
    .getByRole("button", { name: "Close side chats", exact: true })
    .click();
  await expect(tabs).toBeHidden();
  await expect(composer).toHaveValue("Retained parent workspace draft");
  await page
    .getByRole("button", { name: "Open side chats", exact: true })
    .click();
  await expect(tabs.getByRole("tab")).toHaveCount(2);
  await expect(sideTab).toHaveAttribute("aria-selected", "true");
  await expect(question).toHaveValue("Retained side workspace draft");
  await artifactTab.click();
  await expect(
    page.getByRole("complementary", { name: "Artifact", exact: true }),
  ).toContainText("44 bytes");
  await expect(composer).toHaveValue("Retained parent workspace draft");
  expect(fixture.branchSubmissions).toHaveLength(0);
  expect(fixture.submissions).toHaveLength(0);
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`workspace headers and composer docks align while the ${appearance.label} Large right pane resizes and expands`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      plan: true,
      branches: true,
    });
    await page.setViewportSize({ width: 1760, height: 1040 });
    await selectAppearance(page, appearance);
    await page.goto(parentUrl);
    const composer = mainComposer(page);
    await composer.fill("Main workspace draft");
    await page
      .locator('[data-aside-message-id="released-answer"]')
      .getByRole("button", { name: "Ask in side chat", exact: true })
      .click();
    const side = page.getByRole("complementary", {
      name: "Side chats",
      exact: true,
    });
    const question = side.getByRole("textbox", {
      name: "Side-chat question",
      exact: true,
    });
    await question.fill("Side workspace draft");
    const mainHeader = page.locator(
      ".shared-inspector-main > .thread-detail > .shared-work-header.is-compact",
    );
    const rightHeader = side.locator(":scope > .shared-inspector-header");
    const mainDock = page.locator(
      ".shared-inspector-main > .thread-detail > .shared-composer-dock",
    );
    const sideDock = side.locator(
      ".thread-branch-draft > .shared-composer-dock",
    );
    const geometry = async () => {
      const [mainTop, rightTop, mainBottom, rightBottom, mainFrame, sideFrame] =
        await Promise.all([
          mainHeader.boundingBox(),
          rightHeader.boundingBox(),
          mainDock.boundingBox(),
          sideDock.boundingBox(),
          mainDock.locator(".shared-conversation-composer").boundingBox(),
          sideDock.locator(".shared-conversation-composer").boundingBox(),
        ]);
      if (
        !mainTop ||
        !rightTop ||
        !mainBottom ||
        !rightBottom ||
        !mainFrame ||
        !sideFrame
      )
        return { difference: Number.POSITIVE_INFINITY };
      return {
        difference: Math.max(
          Math.abs(mainTop.y - rightTop.y),
          Math.abs(mainTop.y + mainTop.height - rightTop.y - rightTop.height),
          Math.abs(mainBottom.y - rightBottom.y),
          Math.abs(
            mainBottom.y +
              mainBottom.height -
              rightBottom.y -
              rightBottom.height,
          ),
          Math.abs(mainFrame.y - sideFrame.y),
          Math.abs(
            mainFrame.y + mainFrame.height - sideFrame.y - sideFrame.height,
          ),
        ),
        mainTop,
        rightTop,
        mainBottom,
        rightBottom,
        mainFrame,
        sideFrame,
      };
    };
    await expect
      .poll(async () => (await geometry()).difference)
      .toBeLessThanOrEqual(1)
      .finally(async () => {
        await test.info().attach(`workspace-alignment-${appearance.id}.json`, {
          body: JSON.stringify(await geometry(), null, 2),
          contentType: "application/json",
        });
      });
    expect((await mainHeader.boundingBox())!.height).toBeLessThanOrEqual(64);
    expect(
      (await page.locator(".control-header").boundingBox())!.height,
    ).toBeLessThanOrEqual(48);
    const details = mainHeader.locator(".work-header-details");
    const detailsTrigger = details.locator("summary");
    await detailsTrigger.focus();
    await detailsTrigger.press("Enter");
    await expect(details).toHaveAttribute("open", "");
    await expect(
      details.getByText("Personal chat", { exact: true }),
    ).toBeVisible();
    await expect(
      details.getByText("Runtime ready", { exact: true }),
    ).toBeVisible();
    await expect
      .poll(async () => (await geometry()).difference)
      .toBeLessThanOrEqual(1);
    await detailsTrigger.press("Escape");
    await expect(details).not.toHaveAttribute("open", "");
    await expect(detailsTrigger).toBeFocused();
    await expect(side).toBeVisible();
    await page.screenshot({
      path: `output/playwright/workspace-shared-shell-${appearance.id}-large.png`,
    });
    const separator = page.getByRole("separator", {
      name: "Resize inspector",
      exact: true,
    });
    const startingWidth = (await side.boundingBox())!.width;
    await separator.focus();
    for (let index = 0; index < 3; index++) await separator.press("ArrowLeft");
    await expect
      .poll(async () => (await side.boundingBox())!.width)
      .toBeGreaterThan(startingWidth);
    const resizedWidth = (await side.boundingBox())!.width;
    await expect(composer).toHaveValue("Main workspace draft");
    await expect(question).toHaveValue("Side workspace draft");
    const expand = side.getByRole("button", {
      name: "Expand right pane",
      exact: true,
    });
    await expand.focus();
    await expand.press("Enter");
    const restore = side.getByRole("button", {
      name: "Restore right pane",
      exact: true,
    });
    await expect(restore).toBeVisible();
    await expect(separator).toHaveCount(0);
    const workspaceWidth = (await page
      .locator(".shared-inspector-layout")
      .boundingBox())!.width;
    await expect
      .poll(async () => (await side.boundingBox())!.width)
      .toBeGreaterThanOrEqual(workspaceWidth - 1);
    await expect(composer).toHaveCount(1);
    await expect(composer).toHaveValue("Main workspace draft");
    await expect(question).toHaveValue("Side workspace draft");
    await restore.focus();
    await restore.press("Enter");
    await expect(separator).toBeVisible();
    await expect
      .poll(async () =>
        Math.abs((await side.boundingBox())!.width - resizedWidth),
      )
      .toBeLessThanOrEqual(1);
    await expect(composer).toHaveValue("Main workspace draft");
    await expect(question).toHaveValue("Side workspace draft");
    const longDraft = Array.from(
      { length: 60 },
      (_, index) => `Retained side draft line ${index + 1}`,
    ).join("\n");
    await question.fill(longDraft);
    await expect(question).toHaveValue(longDraft);
    await expect(composer).toHaveValue("Main workspace draft");
    await expect(
      side.getByRole("button", { name: "Start side chat", exact: true }),
    ).toBeInViewport({ ratio: 1 });
    const [headerBounds, dockBounds, sideBounds] = await Promise.all([
      rightHeader.boundingBox(),
      sideDock.boundingBox(),
      side.boundingBox(),
    ]);
    expect(dockBounds!.y).toBeGreaterThanOrEqual(
      headerBounds!.y + headerBounds!.height,
    );
    expect(dockBounds!.y + dockBounds!.height).toBeLessThanOrEqual(
      sideBounds!.y + sideBounds!.height + 1,
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(fixture.branchSubmissions).toHaveLength(0);
    expect(fixture.submissions).toHaveLength(0);
    fixture.verify();
  });
}

test("history row Fork opens an independent draft and creates a chat only on its first send", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    plan: true,
    branches: true,
  });
  await page.setViewportSize({ width: 1510, height: 1000 });
  await page.goto(parentUrl);
  const composer = mainComposer(page);
  await composer.fill("Keep this original draft");
  const trigger = page.getByRole("button", {
    name: `Chat actions for ${parentTitle}`,
    exact: true,
  });
  expect(await trigger.evaluate((button) => button.closest("a") === null)).toBe(
    true,
  );
  await trigger.focus();
  await trigger.press("Enter");
  const fork = page.getByRole("menuitem", { name: "Fork chat", exact: true });
  await expect(fork).toBeEnabled();
  await fork.focus();
  await fork.press("Enter");
  const draft = page.getByRole("region", { name: "New fork", exact: true });
  await expect(draft).toBeVisible();
  await expect(page).toHaveURL(parentUrl);
  expect(fixture.branchSubmissions).toHaveLength(0);
  const forkComposer = draft.getByRole("textbox", {
    name: "Message the fork",
    exact: true,
  });
  await expect(forkComposer).toBeFocused();
  await forkComposer.fill("Take a different direction");
  await expect(composer).toHaveValue("Keep this original draft");
  await page.screenshot({
    path: "output/playwright/conversation-fork-draft.png",
  });
  await draft.getByRole("button", { name: "Start fork", exact: true }).click();
  await expect.poll(() => fixture.branchSubmissions.length).toBe(1);
  expect(fixture.branchSubmissions[0]).toMatchObject({
    kind: "fork",
    source_task_id: "browser-plan-task",
    expected_revision: 1,
    request: {
      session_id: null,
      plan_action: null,
      input: [{ text: "Take a different direction" }],
    },
  });
  await expect(page).toHaveURL("/projects/browser-project/chat/browser-fork-1");
  await expect(
    page
      .getByRole("navigation", { name: "Your chat history", exact: true })
      .getByRole("link", { name: `Fork of ${parentTitle}`, exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Independent branch answer", { exact: true }),
  ).toBeVisible();
  expect(fixture.submissions).toHaveLength(0);
  fixture.verify();
});

test.describe("touch history row actions", () => {
  test.use({ hasTouch: true });
  test("menus remain separate from navigation through Rename, Archive, and Restore", async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      plan: true,
      branches: true,
    });
    await page.goto(parentUrl);
    const composer = mainComposer(page);
    await composer.fill("Touch navigation draft");
    await openConversationList(page);
    const history = page
      .getByRole("dialog")
      .getByRole("navigation", { name: "Your chat history", exact: true });
    let trigger = history.getByRole("button", {
      name: `Chat actions for ${parentTitle}`,
      exact: true,
    });
    const row = history.getByRole("link", {
      name: parentTitle,
      exact: true,
    });
    const [rowBounds, triggerBounds] = await Promise.all([
      row.boundingBox(),
      trigger.boundingBox(),
    ]);
    expect(rowBounds!.height).toBeGreaterThanOrEqual(44);
    expect(triggerBounds!.height).toBeGreaterThanOrEqual(44);
    expect(triggerBounds!.width).toBeGreaterThanOrEqual(44);
    await expect(row).toHaveAccessibleDescription(/Updated /u);
    await trigger.tap();
    const rename = page.getByRole("menuitem", {
      name: "Rename",
      exact: true,
    });
    await expect(rename).toBeVisible();
    expect((await rename.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
    await trigger.tap();
    await page.getByRole("menuitem", { name: "Rename", exact: true }).tap();
    const title = history.getByRole("textbox", {
      name: "Chat title",
      exact: true,
    });
    await expect(title).toBeFocused();
    await title.fill("Renamed touch conversation");
    await history.getByRole("button", { name: "Save", exact: true }).tap();
    await expect.poll(() => fixture.threadMutations.length).toBe(1);
    await expect(
      history.getByRole("link", {
        name: "Renamed touch conversation",
        exact: true,
      }),
    ).toBeVisible();
    await expect(page).toHaveURL(parentUrl);
    trigger = history.getByRole("button", {
      name: "Chat actions for Renamed touch conversation",
      exact: true,
    });
    await trigger.tap();
    await page.getByRole("menuitem", { name: "Archive", exact: true }).tap();
    await expect.poll(() => fixture.threadMutations.length).toBe(2);
    await expect(
      history.getByRole("link", {
        name: "Renamed touch conversation",
        exact: true,
      }),
    ).toHaveCount(0);
    await page.getByRole("radio", { name: "Archived", exact: true }).tap();
    await expect(
      history.getByRole("link", {
        name: "Renamed touch conversation",
        exact: true,
      }),
    ).toBeVisible();
    await trigger.tap();
    await page.getByRole("menuitem", { name: "Restore", exact: true }).tap();
    await expect.poll(() => fixture.threadMutations.length).toBe(3);
    await page.getByRole("radio", { name: "Open", exact: true }).tap();
    await expect(
      history.getByRole("link", {
        name: "Renamed touch conversation",
        exact: true,
      }),
    ).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(composer).toHaveValue("Touch navigation draft");
    expect(fixture.branchSubmissions).toHaveLength(0);
    fixture.verify();
  });
});
