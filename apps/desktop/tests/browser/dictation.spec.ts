import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const fixture = {
      events: [] as unknown[],
      calls: [] as string[],
      turn: 1,
      segment: 1,
      model: null as string | null,
      denied: false,
    };
    const transcript = (text: string, final: boolean) => ({
      type: "transcript",
      turn_id: fixture.turn,
      update: {
        segment_id: fixture.segment,
        revision: final ? 2 : 1,
        is_final: final,
        text,
        audio_ms: 2000,
        inference_ms: 10,
      },
    });
    Object.assign(window, {
      dictationFixture: fixture,
      __TAURI_INTERNALS__: {
        invoke: async (
          command: string,
          args: { request?: { action: string } },
        ) => {
          fixture.calls.push(
            command + (args?.request ? `:${args.request.action}` : ""),
          );
          if (command === "dictation_status")
            return { enabled: true, model: fixture.model };
          if (command === "choose_dictation_model") {
            fixture.model = "Tiny English";
            return { enabled: true, model: fixture.model };
          }
          if (command === "start_dictation") {
            fixture.events = fixture.denied
              ? [{ type: "failure", error: "capture_unavailable" }]
              : [
                  { type: "state", phase: "recording" },
                  transcript("Check Linux", false),
                ];
            return "native-session";
          }
          if (command === "poll_dictation") return fixture.events.splice(0);
          if (command === "control_dictation") {
            const action = args.request?.action;
            if (action === "pause")
              return [
                transcript("Check Linux audio", true),
                { type: "state", phase: "paused" },
              ];
            if (action === "resume") {
              fixture.segment += 1;
              fixture.events = [transcript("then Windows", false)];
              return [{ type: "state", phase: "recording" }];
            }
            if (action === "finish_turn") {
              const final = transcript("then Windows too.", true);
              fixture.turn += 1;
              fixture.segment += 1;
              return [
                final,
                { type: "boundary", turn_id: fixture.turn },
                transcript("Next turn", false),
              ];
            }
            if (action === "stop")
              return [
                transcript("Next turn finalized.", true),
                { type: "state", phase: "stopped" },
              ];
            if (action === "abort") {
              fixture.events = [];
              return [{ type: "state", phase: "stopped" }];
            }
          }
          throw new Error(`Unexpected native fixture command: ${command}`);
        },
      },
    });
  });
});

async function startRecording(page: import("@playwright/test").Page) {
  await page
    .getByRole("button", { name: "Start offline dictation", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Choose model…", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Start recording", exact: true })
    .click();
}

test("live composer controls pause for edits and keep the session across an explicit queued send", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=interaction-question");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await startRecording(page);
  await expect(prompt).toHaveValue("Check Linux");
  await expect(prompt).toHaveAttribute("readonly", "");
  await prompt.evaluate((textarea) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", "Competing paste");
    textarea.dispatchEvent(
      new ClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      }),
    );
  });
  await expect(prompt).toHaveValue("Check Linux");
  await expect(
    page.getByText(
      "Recording locally · Pause to edit · Send keeps the microphone on",
      { exact: true },
    ),
  ).toBeVisible();
  await page.screenshot({ path: "output/playwright/dictation-preview.png" });
  await page
    .getByRole("button", { name: "Pause dictation", exact: true })
    .click();
  await expect(prompt).toHaveValue("Check Linux audio");
  await expect(prompt).not.toHaveAttribute("readonly", "");
  await prompt.fill("Check macOS audio");
  await page
    .getByRole("button", { name: "Resume dictation", exact: true })
    .click();
  await expect(prompt).toHaveValue("Check macOS audio then Windows");
  await page
    .getByRole("button", { name: "Add message to Next up", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Next up", exact: true }),
  ).toContainText("Check macOS audio then Windows too.");
  await expect(prompt).toHaveValue("Next turn");
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .click();
  await expect(prompt).toHaveValue("Next turn finalized.");
  await expect(prompt).not.toHaveAttribute("readonly", "");
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as { dictationFixture: { calls: string[] } }
        ).dictationFixture.calls.filter((call) => call === "start_dictation")
          .length,
    ),
  ).toBe(1);
});

test("permission failure keeps typed text and gives microphone recovery guidance", async ({
  page,
}) => {
  await page.goto("/?fixture=interaction-question");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Keep my draft");
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { denied: boolean } }
    ).dictationFixture.denied = true;
  });
  await startRecording(page);
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "system microphone privacy settings" }),
  ).toBeVisible();
  await expect(prompt).toHaveValue("Keep my draft");
  await expect(prompt).not.toHaveAttribute("readonly", "");
  await expect(
    page.getByRole("button", { name: "Stop dictation", exact: true }),
  ).toHaveCount(0);
});
