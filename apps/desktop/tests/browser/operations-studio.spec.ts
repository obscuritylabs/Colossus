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

test("settings dropdowns use styled app-owned menus with keyboard support", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();

  await expect(page.locator("select")).toHaveCount(0);
  const accessProfile = page.getByRole("combobox", {
    name: "Access profile",
    exact: true,
  });
  await accessProfile.click();

  const listbox = page.getByRole("listbox");
  await expect(listbox).toBeVisible();
  await expect(listbox.getByRole("option")).toHaveCount(5);
  await expect(
    listbox.getByRole("option", { name: "Allow all", exact: true }),
  ).toHaveAttribute("aria-selected", "true");

  await accessProfile.press("ArrowUp");
  await accessProfile.press("Enter");
  await expect(accessProfile).toContainText("Development");
  await accessProfile.click();
  await listbox.getByRole("option", { name: "Allow all", exact: true }).click();
  await expect(accessProfile).toContainText("Allow all");

  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Telemetry", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit Local collector", exact: true })
    .click();
  const protocol = page.getByRole("combobox", {
    name: "Protocol",
    exact: true,
  });
  await protocol.click();
  await expect(page.getByRole("listbox").getByRole("option")).toHaveCount(2);

  const results = await new AxeBuilder({ page })
    .include(".app-select-popover")
    .analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);

  await protocol.press("Escape");
  await expect(page.getByRole("listbox")).toHaveCount(0);
});

test("managed settings expose complete catalog editors without horizontal overflow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();

  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByRole("button", { name: "Edit primary", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "Reasoning effort" }),
  ).toBeVisible();
  await expect(page.getByText("Image inputs", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Close model editor" }).click();

  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit primary-provider", exact: true })
    .click();
  await expect(page.getByPlaceholder("Adapter default")).toBeVisible();
  await page.getByRole("button", { name: "Close provider editor" }).click();

  await page.getByRole("button", { name: "MCP", exact: true }).click();
  await page
    .getByRole("button", { name: "Edit splunk-search", exact: true })
    .click();
  for (const label of [
    "Static headers",
    "Research tool projections",
    "Timeout (ms)",
    "Maximum output (bytes)",
  ]) {
    await expect(page.getByLabel(label, { exact: true })).toBeVisible();
  }
  await expect(
    page.getByText("Credential headers", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("OAuth client configuration", { exact: true }),
  ).toBeVisible();

  const pane = page.locator(".managed-settings-shell");
  const bounds = await pane.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
});

test("session snapshots and every durable record family are listable and inspectable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Snapshots", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Context snapshots" }),
  ).toBeVisible();
  await expect(page.getByText(/Messages 1–18/u)).toBeVisible();
  await page.getByRole("button", { name: "View snapshot" }).first().click();
  await expect(
    page.getByText("Context snapshot", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Pinned facts", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Open tasks", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Back to thread details" }).click();

  await page.getByRole("button", { name: "Resources", exact: true }).click();
  const resources = page.getByRole("region", { name: "Resources" });
  for (const label of [
    "Delegated agents",
    "Goals",
    "Tasks",
    "Key decisions",
    "Memories",
    "Research",
  ]) {
    await expect(resources.getByText(label, { exact: true })).toBeVisible();
  }
  await resources.getByText("Key decisions", { exact: true }).click();
  await resources
    .getByRole("button", { name: /Keep execution boundary unchanged/u })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Keep execution boundary unchanged",
      exact: true,
    }),
  ).toBeVisible();
});

test("required app-owned dropdowns retain form validation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Models", exact: true }).click();
  await page.getByRole("button", { name: "Add model", exact: true }).click();

  const provider = page.getByRole("combobox", {
    name: "Provider",
    exact: true,
  });
  await provider.click();
  await page
    .getByRole("option", { name: "Select provider", exact: true })
    .click();
  await page.getByRole("textbox", { name: "Label" }).fill("Validation model");
  await page.getByRole("textbox", { name: "Profile" }).fill("validation-model");
  await page
    .getByRole("textbox", { name: "Model identifier" })
    .fill("fixture-validation");
  await page.getByRole("button", { name: "Save revision" }).click();

  await expect(
    page.getByRole("heading", { name: "Add model", exact: true }),
  ).toBeVisible();
  await expect(provider).toBeFocused();
  await expect(provider).toHaveAttribute("aria-invalid", "true");
});

test("Workspace archive and restore update the sidebar and confirm the outcome", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  await page.getByRole("button", { name: "Manage Workspaces" }).click();
  await page.getByRole("button", { name: "Archive Colossus" }).click();

  await expect(page.getByRole("status")).toContainText("Archived Colossus.");
  await expect(
    page.locator(".space-shelf.is-active", { hasText: "Colossus" }),
  ).toHaveCount(0);
  await expect(page.locator(".space-shelf.is-active")).toContainText(
    "Research Lab",
  );

  await page.getByRole("button", { name: "Manage Workspaces" }).click();
  const restore = page.getByRole("button", { name: "Restore Colossus" });
  await expect(restore).toBeVisible();
  await restore.click();

  await expect(page.getByRole("status")).toContainText(
    "Restored Colossus. Select it when you’re ready.",
  );
  await expect(
    page.getByRole("button", { name: "Restore Colossus" }),
  ).toHaveCount(0);
});

test("Research settings expose depth and evidence choices and restore focus", async ({
  page,
}) => {
  const researchMode = page.getByRole("radio", { name: "Research" });
  await page
    .locator(".mode-switch")
    .getByText("Research", { exact: true })
    .click();
  await expect(researchMode).toBeChecked();

  const settingsTrigger = page.getByLabel(
    "Research controls, sources This Workspace",
  );
  await settingsTrigger.click();

  await expect(
    page.getByRole("heading", { name: "Research settings" }),
  ).toBeVisible();
  await expect(page.getByRole("radio", { name: "Standard" })).toBeChecked();
  await expect(
    page.getByRole("checkbox", { name: /This Workspace/u }),
  ).toBeChecked();

  await page.locator(".research-depth-option", { hasText: "Deep" }).click();
  await page.locator(".research-source-option", { hasText: "Web" }).click();
  await expect(page.getByRole("radio", { name: "Deep" })).toBeChecked();
  await expect(
    page.getByLabel("Research controls, sources This Workspace, Web"),
  ).toBeVisible();

  await page.getByRole("button", { name: "Close research settings" }).click();
  const updatedTrigger = page.getByLabel(
    "Research controls, sources This Workspace, Web",
  );
  await expect(updatedTrigger).toBeFocused();

  await updatedTrigger.click();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("heading", { name: "Research settings" }),
  ).toHaveCount(0);
  await expect(updatedTrigger).toBeFocused();
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

test("follow-ups can be queued, edited, deleted, and used to redirect active work", async ({
  page,
}) => {
  await page.goto("/?fixture=interaction-question");

  const prompt = page.getByLabel("Prompt", { exact: true });
  await expect(prompt).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Add message to Next up" }),
  ).toBeVisible();

  await prompt.fill("Check the Windows path too");
  await page.getByRole("button", { name: "Add message to Next up" }).click();

  const nextUp = page.getByRole("region", { name: "Next up" });
  await expect(nextUp).toContainText("Check the Windows path too");
  await nextUp.getByRole("button", { name: "Edit queued message" }).click();
  const editor = nextUp.getByLabel("Edit queued message");
  await editor.fill("Check Windows and Linux paths");
  await nextUp.getByRole("button", { name: "Save" }).click();
  await expect(nextUp).toContainText("Check Windows and Linux paths");
  await nextUp.getByRole("button", { name: "Delete queued message" }).click();
  await expect(nextUp).toHaveCount(0);

  await prompt.fill("Focus on the cancellation race first");
  await page.getByRole("button", { name: "Redirect current response" }).click();

  await expect(page.getByRole("region", { name: "Next up" })).toHaveCount(0);
  await expect(
    page.getByText("Focus on the cancellation race first", { exact: true }),
  ).toBeVisible();
});

test("conversation follow pauses for reading and resumes on demand or submit", async ({
  page,
}) => {
  const feed = page.locator(".work-feed-scroll");
  await feed.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });

  const jumpToLatest = page.getByRole("button", { name: "Jump to latest" });
  await expect(jumpToLatest).toBeVisible();
  await jumpToLatest.click();
  await expect(jumpToLatest).toHaveCount(0);
  await expect
    .poll(() =>
      feed.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight,
      ),
    )
    .toBeLessThanOrEqual(2);

  await feed.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
  });
  await expect(jumpToLatest).toBeVisible();
  await page.getByLabel("Prompt", { exact: true }).fill("Follow this message");
  await page.getByRole("button", { name: "Add message to Next up" }).click();
  await expect(jumpToLatest).toHaveCount(0);
});

test("Colossus responses expose copy confirmation", async ({ page }) => {
  const copyResponse = page
    .getByRole("button", { name: "Copy Colossus response" })
    .first();
  await expect(copyResponse).toBeVisible();
  await copyResponse.click();
  await expect(
    page.getByRole("button", { name: "Copied Colossus response" }).first(),
  ).toBeVisible();
});

test("plan titles open the rendered plan and previews do not leak Markdown syntax", async ({
  page,
}) => {
  await page.goto("/?fixture=plan-workflow");
  await page
    .getByRole("button", { name: "Close details drawer", exact: true })
    .click();
  await page.getByRole("button", { name: "Plans", exact: true }).click();

  const card = page.locator(".session-plan-list article");
  await expect(
    card.getByRole("heading", { name: "Desktop Plan workflow" }),
  ).toBeVisible();
  await expect(card).not.toContainText("## Desktop Plan workflow");

  await card
    .getByRole("button", { name: "Plan the Desktop release workflow" })
    .click();
  const details = page.getByLabel("Thread details", { exact: true });
  await expect(
    details.getByRole("heading", {
      name: "Plan the Desktop release workflow",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    details.getByText("Rendered from the durable plan output"),
  ).toBeVisible();
});

test("Session Map expands canonical resources and opens their inspector", async ({
  page,
}) => {
  const feed = page.locator(".work-feed-scroll");
  await feed.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });

  await page.getByRole("button", { name: "Topology", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Session map", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("form", { name: "Send a prompt" })).toHaveCount(
    0,
  );
  await expect(page.locator(".session-map-primary")).toBeVisible();
  await expect
    .poll(() => feed.evaluate((element) => element.scrollTop))
    .toBe(0);

  const memories = page.getByRole("button", { name: /Memories 3/u });
  await expect(memories).toHaveAttribute("aria-expanded", "true");
  await expect(
    page.getByRole("button", {
      name: /Use Rust 1\.96 and edition 2024/u,
    }),
  ).toBeVisible();

  const goals = page.getByRole("button", { name: /Goals 2/u });
  await goals.click();
  await expect(goals).toHaveAttribute("aria-expanded", "true");
  await expect(
    page.getByRole("button", { name: /Review workspace architecture/u }),
  ).toBeVisible();

  await expect(page.locator(".react-flow")).toBeVisible();
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  await expect(goals).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".session-map-primary")).toBeVisible();

  const feedTopBeforeInspection = await feed.evaluate(
    (element) => element.scrollTop,
  );
  await page
    .getByRole("button", { name: /Use Rust 1\.96 and edition 2024/u })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Use Rust 1.96 and edition 2024 for implementation work.",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByText("Repository", { exact: true })).toBeVisible();
  await expect(page.getByText("100%", { exact: true })).toBeVisible();
  await expect
    .poll(() => feed.evaluate((element) => element.scrollTop))
    .toBe(feedTopBeforeInspection);
});

test("Session Map keeps its root visible without horizontal overflow at compact widths", async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 760 });
  await page.goto(FIXTURE);
  await page.getByRole("button", { name: "Topology", exact: true }).click();

  const stage = page.locator(".session-map-stage");
  const primary = page.locator(".session-map-primary");
  await expect(primary).toBeVisible();
  await expect
    .poll(() =>
      stage.evaluate((element) => element.scrollWidth - element.clientWidth),
    )
    .toBe(0);

  const stageBox = await stage.boundingBox();
  const primaryBox = await primary.boundingBox();
  expect(stageBox).not.toBeNull();
  expect(primaryBox).not.toBeNull();
  expect(primaryBox!.x).toBeGreaterThanOrEqual(stageBox!.x);
  expect(primaryBox!.x + primaryBox!.width).toBeLessThanOrEqual(
    stageBox!.x + stageBox!.width,
  );
});

test("Session Map renders its topology with React Flow SVG edges", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(FIXTURE);
  await page.getByRole("button", { name: "Topology", exact: true }).click();

  const flow = page.locator(".react-flow");
  const primary = page.locator(".session-map-primary");
  const firstFamily = page.locator(".session-map-family").first();
  await expect(flow).toBeVisible();
  await expect(page.locator(".react-flow__edge-path")).toHaveCount(13);
  await expect(page.locator(".session-map-network")).toHaveCount(0);
  await expect(page.locator(".session-map-trunk")).toHaveCount(0);

  const primaryBox = await primary.boundingBox();
  const familyBox = await firstFamily.boundingBox();
  expect(primaryBox).not.toBeNull();
  expect(familyBox).not.toBeNull();
  expect(familyBox!.x).toBeGreaterThan(primaryBox!.x + primaryBox!.width);

  const stageBox = await page.locator(".session-map-stage").boundingBox();
  const flowBox = await flow.boundingBox();
  expect(stageBox).not.toBeNull();
  expect(flowBox).not.toBeNull();
  expect(Math.abs(flowBox!.x - stageBox!.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(flowBox!.y - stageBox!.y)).toBeLessThanOrEqual(1);
  expect(stageBox!.width - flowBox!.width).toBeLessThanOrEqual(2);
  expect(stageBox!.height - flowBox!.height).toBeLessThanOrEqual(2);
});

test("Session Map loads with the native frozen-prototype boundary", async ({
  page,
}) => {
  await page.addInitScript(() => Object.freeze(Object.prototype));
  await page.reload();
  await page.getByRole("button", { name: "Topology", exact: true }).click();

  await expect(page.locator(".react-flow")).toBeVisible();
  await expect(page.getByRole("button", { name: /Goals 2/u })).toBeVisible();
});

test("Session Activity synchronizes timeline, feed, and released inspector content", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${FIXTURE}&view=activity`);

  await expect(
    page.getByRole("heading", { name: "Session activity", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("form", { name: "Send a prompt" })).toHaveCount(
    0,
  );
  const timeline = page.getByRole("region", {
    name: "Session activity timeline",
  });
  await expect(timeline).toBeVisible();
  await expect(timeline.getByText("Agent", { exact: true })).toBeVisible();
  await expect(timeline.getByText("Tools", { exact: true })).toBeVisible();
  await expect(timeline.getByText("System", { exact: true })).toBeVisible();

  const toolRow = page
    .locator(".activity-row", { hasText: "shell.exec" })
    .first();
  await toolRow.click();
  await expect(toolRow).toHaveAttribute("data-selected", "true");
  const inspector = page.getByRole("complementary", {
    name: "Activity inspector",
  });
  await expect(inspector).toContainText("shell.exec");
  await inspector.getByRole("button", { name: "Input", exact: true }).click();
  await expect(inspector.locator("pre")).toContainText("git status --short");

  const timelineTool = page
    .getByRole("button", { name: /filesystem\.search,/u })
    .first();
  await timelineTool.click();
  await expect(inspector).toContainText("filesystem.search");
  await expect(
    page.locator('.activity-row[data-selected="true"]'),
  ).toContainText("filesystem.search");

  const follow = timeline.getByRole("button", {
    name: "Follow live activity",
  });
  await expect(follow).toHaveAttribute("aria-pressed", "true");

  await timeline.getByRole("button", { name: "Zoom in" }).click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");

  await timeline.getByRole("button", { name: "Select time range" }).click();
  const firstTrack = timeline.locator(".activity-timeline-track").first();
  const trackBox = await firstTrack.boundingBox();
  expect(trackBox).not.toBeNull();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.55,
    trackBox!.y + trackBox!.height / 2,
    { steps: 5 },
  );
  await page.mouse.up();
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(3);

  const zoomToRange = timeline.getByRole("button", {
    name: "Zoom to selected time range",
  });
  await expect(zoomToRange).toBeEnabled();
  await zoomToRange.click();
  await follow.click();
  await expect(follow).toHaveAttribute("aria-pressed", "true");

  await timeline.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  await page.keyboard.press("End");
  await expect(follow).toHaveAttribute("aria-pressed", "true");

  const results = await new AxeBuilder({ page })
    .include(".session-activity")
    .analyze();
  const blockingViolations = results.violations.filter((violation) =>
    ["critical", "serious"].includes(violation.impact ?? ""),
  );
  expect(blockingViolations).toEqual([]);
});

test("Session Activity timeline navigation supports mouse, wheel, range, fit, and keyboard input", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${FIXTURE}&view=activity`);

  const timeline = page.getByRole("region", {
    name: "Session activity timeline",
  });
  const axis = timeline.locator(".activity-timeline-axis span");
  const firstTrack = timeline.locator(".activity-timeline-track").first();
  const trackBox = await firstTrack.boundingBox();
  expect(trackBox).not.toBeNull();

  const follow = timeline.getByRole("button", {
    name: "Follow live activity",
  });
  const fit = timeline.getByRole("button", {
    name: "Fit entire timeline",
  });
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await expect(fit).toContainText("1.0×");

  await timeline.getByRole("button", { name: "Zoom in" }).click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  await expect(fit).not.toContainText("1.0×");

  const beforeDragPan = await axis.allTextContents();
  await timeline.getByRole("button", { name: "Pan timeline" }).click();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.68,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.42,
    trackBox!.y + trackBox!.height / 2,
    { steps: 6 },
  );
  await page.mouse.up();
  expect(await axis.allTextContents()).not.toEqual(beforeDragPan);

  await timeline.getByRole("button", { name: "Select time range" }).click();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.76,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.3,
    trackBox!.y + trackBox!.height / 2,
    { steps: 6 },
  );
  await page.mouse.up();
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(3);
  await expect(timeline.getByRole("status")).toContainText("–");

  await timeline.focus();
  await page.keyboard.press("Escape");
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(0);

  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.4,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.4 + 2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.up();
  await expect(timeline.locator(".activity-timeline-selection")).toHaveCount(0);

  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    trackBox!.x + trackBox!.width * 0.55,
    trackBox!.y + trackBox!.height / 2,
    { steps: 5 },
  );
  await page.mouse.up();
  const zoomToRange = timeline.getByRole("button", {
    name: "Zoom to selected time range",
  });
  await zoomToRange.click();
  await expect(fit).not.toContainText("1.0×");
  await fit.click();
  await expect(fit).toContainText("1.0×");
  await expect(
    timeline.getByRole("button", { name: "Zoom out" }),
  ).toBeDisabled();

  await page.mouse.move(
    trackBox!.x + trackBox!.width / 2,
    trackBox!.y + trackBox!.height / 2,
  );
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -160);
  await page.keyboard.up("Control");
  await expect(fit).not.toContainText("1.0×");

  const beforeWheelPan = await axis.allTextContents();
  await firstTrack.dispatchEvent("wheel", {
    clientX: trackBox!.x + trackBox!.width / 2,
    deltaX: 180,
    deltaY: 0,
  });
  expect(await axis.allTextContents()).not.toEqual(beforeWheelPan);

  await timeline.focus();
  await page.keyboard.press("Home");
  await expect(fit).toContainText("1.0×");
  await page.keyboard.press("+");
  await expect(fit).not.toContainText("1.0×");
  const beforeKeyboardPan = await axis.allTextContents();
  await page.keyboard.press("ArrowLeft");
  expect(await axis.allTextContents()).not.toEqual(beforeKeyboardPan);
  await page.keyboard.press("End");
  await expect(follow).toHaveAttribute("aria-pressed", "true");
});

test("Session Activity Follow tracks new records and manual navigation holds its viewport", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${FIXTURE}&view=activity&activityLive=1`);

  const timeline = page.getByRole("region", {
    name: "Session activity timeline",
  });
  const axis = timeline.locator(".activity-timeline-axis span");
  const follow = timeline.getByRole("button", {
    name: "Follow live activity",
  });
  const initialAxis = await axis.allTextContents();
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByText("27 curated events", { exact: true }),
  ).toBeVisible();

  await expect(
    timeline.getByRole("button", { name: /Live checkpoint,/u }),
  ).toBeVisible({ timeout: 7_000 });
  await expect(
    page.getByText("28 curated events", { exact: true }),
  ).toBeVisible();
  expect(await axis.allTextContents()).not.toEqual(initialAxis);

  await timeline.getByRole("button", { name: "Zoom in" }).click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  const heldAxis = await axis.allTextContents();

  await expect(
    page.getByText("29 curated events", { exact: true }),
  ).toBeVisible({ timeout: 7_000 });
  await expect(
    timeline.getByRole("button", { name: /Live response,/u }),
  ).toHaveCount(0);
  expect(await axis.allTextContents()).toEqual(heldAxis);

  await follow.click();
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await expect(
    timeline.getByRole("button", { name: /Live response,/u }),
  ).toBeVisible();
  expect(await axis.allTextContents()).not.toEqual(heldAxis);
});

test("Session Activity search, filters, live state, and compact stacking remain functional", async ({
  page,
}) => {
  await page.setViewportSize({ width: 740, height: 780 });
  await page.goto(`${FIXTURE}&view=activity`);

  const search = page.getByRole("searchbox", {
    name: "Search session activity",
  });
  await search.fill("denied");
  await expect(page.locator(".activity-row")).toHaveCount(1);
  await expect(page.locator(".activity-row")).toContainText("denied");

  await page.getByRole("button", { name: "Filter", exact: true }).click();
  const filters = page.getByRole("dialog", { name: "Activity filters" });
  await filters.getByRole("checkbox", { name: "Tools", exact: true }).check();
  await filters.getByRole("checkbox", { name: "Failed", exact: true }).check();
  await expect(page.getByRole("button", { name: "Filter (2)" })).toBeVisible();
  await expect(page.locator(".activity-row")).toHaveCount(1);

  const live = page.getByRole("button", { name: "Live", exact: true });
  await live.click();
  await expect(
    page.getByRole("button", { name: "Paused", exact: true }),
  ).toHaveAttribute("aria-pressed", "false");

  const feedBox = await page
    .getByRole("region", { name: "Session activity feed" })
    .boundingBox();
  const inspectorBox = await page
    .getByRole("complementary", { name: "Activity inspector" })
    .boundingBox();
  expect(feedBox).not.toBeNull();
  expect(inspectorBox).not.toBeNull();
  expect(inspectorBox!.y).toBeGreaterThanOrEqual(feedBox!.y + feedBox!.height);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    ),
  ).toBe(0);
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

test("Workspace navigation and approvals are keyboard-operable", async ({
  page,
}) => {
  const navigationTrigger = page.getByRole("button", {
    name: "Open work navigation",
  });
  await navigationTrigger.click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  await expect(navigation).toBeVisible();
  await expect(
    navigation.getByRole("button", { name: "Close navigation" }),
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

test("responsive Workspace navigation remains reachable from catalog surfaces", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "Capabilities", exact: true })
    .click();

  const navigationTrigger = page.getByRole("button", {
    name: "Open Workspace navigation",
  });
  await expect(navigationTrigger).toBeVisible();
  await navigationTrigger.click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  const close = navigation.getByRole("button", { name: "Close navigation" });
  await expect(close).toBeVisible();
  await close.click();
  await expect(navigation).toHaveCount(0);
  await expect(navigationTrigger).toBeFocused();
});

test("desktop Workspace sidebar resizes and remembers its width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const sidebar = page.locator(".work-sidebar");
  const resizeHandle = page.getByRole("separator", {
    name: "Resize Workspace sidebar",
  });
  const initialBox = await sidebar.boundingBox();
  const handleBox = await resizeHandle.boundingBox();
  expect(initialBox).not.toBeNull();
  expect(handleBox).not.toBeNull();

  await page.mouse.move(
    handleBox!.x + handleBox!.width / 2,
    handleBox!.y + handleBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(handleBox!.x + 84, handleBox!.y + 120, { steps: 5 });
  await page.mouse.up();

  await expect
    .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
    .toBeGreaterThan(initialBox!.width + 60);
  const resizedWidth = (await sidebar.boundingBox())!.width;

  await page.reload();
  await expect
    .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
    .toBeCloseTo(resizedWidth, 0);

  await resizeHandle.focus();
  await page.keyboard.press("ArrowLeft");
  await expect
    .poll(async () => (await sidebar.boundingBox())?.width ?? 0)
    .toBeCloseTo(resizedWidth - 8, 0);
});

test("collapsing the active Workspace keeps the remaining navigation pinned", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const sidebar = page.locator(".work-sidebar");
  const threadStack = page.locator(".space-thread-stack");
  const footer = page.locator(".space-sidebar-footer");
  const footerBefore = await footer.boundingBox();
  const sidebarBox = await sidebar.boundingBox();
  expect(footerBefore).not.toBeNull();
  expect(sidebarBox).not.toBeNull();

  await page.getByRole("button", { name: "Collapse Colossus threads" }).click();

  await expect(threadStack).toHaveAttribute("aria-hidden", "true");
  await expect(
    page.getByRole("button", { name: "Expand Colossus threads" }),
  ).toBeVisible();
  await expect
    .poll(async () => (await footer.boundingBox())?.y ?? 0)
    .toBeCloseTo(footerBefore!.y, 0);

  const footerAfter = await footer.boundingBox();
  expect(footerAfter).not.toBeNull();
  expect(
    sidebarBox!.y + sidebarBox!.height - (footerAfter!.y + footerAfter!.height),
  ).toBeLessThanOrEqual(14);
});

test("Workspace folders disclose thread summaries without switching context", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const activeSpace = page.locator(".space-shelf.is-active");
  await expect(activeSpace).toContainText("Colossus");

  await page
    .getByRole("button", { name: "Expand Research Lab threads" })
    .click();
  await expect(page.getByText("Review source provenance")).toBeVisible();
  await expect(activeSpace).toContainText("Colossus");

  await page
    .getByRole("button", { name: "Expand Proposal Studio threads" })
    .click();
  await expect(page.getByText("Resolve compliance findings")).toBeVisible();
  await expect(page.getByText("Review source provenance")).toBeVisible();

  await page.getByRole("button", { name: "Research Lab", exact: true }).click();
  await expect(activeSpace).toContainText("Research Lab");
  await expect(
    page.getByRole("button", { name: "New thread in Research Lab" }),
  ).toBeVisible();
});

test("selected Workspace keeps its create action on the trailing edge", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  const activeSpace = page.locator(".space-shelf.is-active");
  const disclosure = activeSpace.getByRole("button", {
    name: "Collapse Colossus threads",
  });
  const create = activeSpace.getByRole("button", {
    name: "New thread in Colossus",
  });
  const [disclosureBox, createBox] = await Promise.all([
    disclosure.boundingBox(),
    create.boundingBox(),
  ]);

  expect(disclosureBox).not.toBeNull();
  expect(createBox).not.toBeNull();
  expect(createBox!.x).toBeGreaterThan(disclosureBox!.x + disclosureBox!.width);
});

test("terminal threads can be archived from the Workspace sidebar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  const archive = page.getByRole("button", {
    name: "Archive Audit ipc boundary",
  });
  await expect(archive).toBeEnabled();
  await archive.click();

  await expect(archive).toHaveCount(0);
  await expect(
    page.getByText("Audit ipc boundary", { exact: true }),
  ).toHaveCount(0);
});

test("threads can be pinned, persisted, and returned to their normal group", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(FIXTURE);

  await page.getByText("Audit ipc boundary", { exact: true }).hover();
  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  await page.getByRole("button", { name: "Pin Audit ipc boundary" }).click();

  const pinned = page.locator(".work-group").filter({
    has: page.getByRole("heading", { name: "Pinned", exact: true }),
  });
  await expect(pinned).toContainText("Audit ipc boundary");
  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Unpin Audit ipc boundary" }),
  ).toHaveAttribute("aria-pressed", "true");

  await page.reload();
  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Unpin Audit ipc boundary" }),
  ).toBeAttached();

  await page.getByRole("button", { name: "Unpin Audit ipc boundary" }).click();
  await expect(pinned).toContainText("Harden desktop agent bootstrap");
  await expect(pinned).not.toContainText("Audit ipc boundary");

  await page
    .getByRole("button", {
      name: "Thread actions for Harden desktop agent bootstrap",
    })
    .click();
  await page
    .getByRole("button", { name: "Unpin Harden desktop agent bootstrap" })
    .click();
  await expect(pinned).toHaveCount(0);

  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Pinned", exact: true }),
  ).toHaveCount(0);
});

test("pinning remains available while the startup connection settles", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto(`${FIXTURE}&connecting=1`);

  await page
    .getByRole("button", {
      name: "Thread actions for Audit ipc boundary",
    })
    .click();
  const pin = page.getByRole("button", { name: "Pin Audit ipc boundary" });
  await expect(pin).toBeEnabled();
  await pin.click();

  const pinned = page.locator(".work-group").filter({
    has: page.getByRole("heading", { name: "Pinned", exact: true }),
  });
  await expect(pinned).toContainText("Audit ipc boundary");
});

test("thread actions survive a WebKit blur without a related target", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });

  const actions = page.getByRole("button", {
    name: "Thread actions for Audit ipc boundary",
  });
  await actions.click();
  const pin = page.getByRole("button", { name: "Pin Audit ipc boundary" });
  await pin.evaluate((button) => {
    const menu = button.closest("details");
    const summary = menu?.querySelector("summary") ?? null;
    if (menu === null || summary === null) {
      throw new Error("thread actions menu is unavailable");
    }
    summary.dispatchEvent(
      new FocusEvent("blur", { bubbles: true, relatedTarget: null }),
    );
    button.focus();
  });
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );

  await expect(pin).toBeVisible();
  await pin.click();
  const pinned = page.locator(".work-group").filter({
    has: page.getByRole("heading", { name: "Pinned", exact: true }),
  });
  await expect(pinned).toContainText("Audit ipc boundary");
});

test("search scope is part of the search control", async ({ page }) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  const search = navigation.getByRole("searchbox", {
    name: "Search threads",
  });
  const scope = navigation.getByLabel("Thread search scope");

  await expect(search).toHaveAttribute("placeholder", "Search threads");
  await expect(
    scope.getByRole("button", { name: "This Workspace", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");

  await scope
    .getByRole("button", { name: "All Workspaces", exact: true })
    .click();

  await expect(
    scope.getByRole("button", { name: "All Workspaces", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    scope.getByRole("button", { name: "This Workspace", exact: true }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(search).toBeFocused();
});

test("Thread details lists released participants and returns focus on close", async ({
  page,
}) => {
  const detailsTrigger = page.getByRole("button", {
    name: "Open thread details",
  });
  await detailsTrigger.click();

  const details = page.getByRole("dialog", { name: "Thread details" });
  await expect(details).toBeVisible();
  await expect(details).toContainText("Atlas");
  await expect(details).toContainText("Builder");
  await expect(details).toContainText("Sentinel");
  await expect(details).toContainText("Scribe");
  await expect(details).toContainText("bootstrap.rs");

  await details.getByRole("button", { name: "Close details drawer" }).click();
  await expect(details).toHaveCount(0);
  await expect(detailsTrigger).toBeFocused();
});

test("Workspace startup keeps search and navigation responsive", async ({
  page,
}) => {
  await page.goto(`${FIXTURE}&spaceStartup=1`);
  await page.getByRole("button", { name: "Open work navigation" }).click();

  const navigation = page.getByRole("dialog", {
    name: "Workspace navigation",
  });
  await expect(
    navigation.getByText("Research Lab", { exact: true }),
  ).toBeVisible();
  await expect(navigation.getByText("Starting", { exact: true })).toBeVisible();
  await expect(
    navigation.getByRole("searchbox", { name: "Search threads" }),
  ).toBeEnabled();
  await expect(
    navigation.getByRole("button", { name: "Capabilities", exact: true }),
  ).toBeEnabled();
  await expect(
    navigation.getByRole("button", { name: "New thread in Research Lab" }),
  ).toBeDisabled();
  await expect(
    navigation.locator('.space-shelf.is-active[aria-busy="true"]'),
  ).toBeVisible();
  await expect(
    navigation.locator('.space-shelf-identity[title="~/tools/research-lab"]'),
  ).toBeVisible();
});

test("follow-up prompts remain in the same work conversation", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "New thread in Colossus" })
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
    name: "Workspace navigation",
  });
  await expect(
    workNavigation.locator(".work-item").filter({ hasText: opening }),
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
