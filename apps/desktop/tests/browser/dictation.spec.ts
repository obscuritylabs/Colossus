import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const fixture = {
      events: [] as unknown[],
      calls: [] as string[],
      turn: 1,
      segment: 1,
      model: null as string | null,
      denied: false,
      initialText: "Check Linux",
      level: null as number | null,
      levelIndex: 0,
      starting: false,
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
              : fixture.starting
                ? [{ type: "state", phase: "starting" }]
                : [
                    { type: "state", phase: "recording" },
                    transcript(fixture.initialText, false),
                  ];
            return "native-session";
          }
          if (command === "poll_dictation") {
            const envelope = [
              0, 0, 25, 100, 150, 190, 130, 65, 30, 170, 225, 170, 100, 10, 0,
              0,
            ];
            const level =
              fixture.level ??
              envelope[fixture.levelIndex++ % envelope.length]!;
            return [...fixture.events.splice(0), { type: "level", level }];
          }
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

test("the recording strip follows input, freezes on pause, and remains usable with reduced motion", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/?fixture=interaction-question");
  await startRecording(page);
  const strip = page.getByRole("group", {
    name: "Dictation recording",
    exact: true,
  });
  const meter = page.getByRole("meter", {
    name: "Microphone input level",
    exact: true,
  });
  await expect(strip.getByText("Listening", { exact: true })).toBeVisible();
  await expect(meter).toHaveAttribute("data-motion", "full");
  await expect
    .poll(async () => Number(await meter.getAttribute("aria-valuenow")))
    .toBeGreaterThan(40);
  await expect(strip.locator(".dictation-recording-clock")).toHaveAttribute(
    "aria-label",
    /^(?:[2-9]|[1-9][0-9]+) seconds recorded$/,
  );
  expect(
    (await new AxeBuilder({ page }).include(".dictation-recording").analyze())
      .violations,
  ).toEqual([]);
  await page.screenshot({ path: "output/playwright/mic-listening-dark.png" });
  await strip.screenshot({ path: "output/playwright/mic-listening-strip.png" });
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { level: number } }
    ).dictationFixture.level = 0;
  });
  await expect(meter).toHaveAttribute("aria-valuenow", "0");
  await page
    .getByRole("button", { name: "Pause dictation", exact: true })
    .click();
  await expect(strip.getByText("Paused", { exact: true })).toBeVisible();
  const clock = await strip
    .locator(".dictation-recording-clock")
    .getAttribute("aria-label");
  await expect(meter).toHaveAttribute("aria-valuenow", "0");
  await page.setViewportSize({ width: 880, height: 640 });
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.screenshot({ path: "output/playwright/mic-paused-narrow.png" });
  await page.waitForTimeout(1100);
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { level: number } }
    ).dictationFixture.level = 200;
  });
  await expect(strip.locator(".dictation-recording-clock")).toHaveAttribute(
    "aria-label",
    clock!,
  );
  await page
    .getByRole("button", { name: "Resume dictation", exact: true })
    .click();
  await expect(meter).toHaveAttribute("data-motion", "reduced");
  await expect
    .poll(async () => Number(await meter.getAttribute("aria-valuenow")))
    .toBeGreaterThan(40);
  const box = await strip.boundingBox();
  const pause = await page
    .getByRole("button", { name: "Pause dictation", exact: true })
    .boundingBox();
  const stop = await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .boundingBox();
  expect(
    box &&
      pause &&
      stop &&
      pause.x >= box.x &&
      stop.x + stop.width <= box.x + box.width,
  ).toBeTruthy();
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(
    (await new AxeBuilder({ page }).include(".dictation-recording").analyze())
      .violations,
  ).toEqual([]);
  await page.screenshot({ path: "output/playwright/mic-listening-light.png" });
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .click();
  await expect(strip).toHaveCount(0);
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).not.toHaveAttribute("readonly", "");
  expect(errors).toEqual([]);
});

test("steady room noise settles into dots while louder speech still moves the meter", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=interaction-question");
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { level: number } }
    ).dictationFixture.level = 84;
  });
  await startRecording(page);
  const meter = page.getByRole("meter", {
    name: "Microphone input level",
    exact: true,
  });
  await expect
    .poll(async () => Number(await meter.getAttribute("aria-valuenow")))
    .toBeGreaterThan(0);
  await expect(meter).toHaveAttribute("aria-valuenow", "0");
  await expect(page.getByText("Listening", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { level: number } }
    ).dictationFixture.level = 200;
  });
  await expect
    .poll(async () => Number(await meter.getAttribute("aria-valuenow")))
    .toBeGreaterThan(40);
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { level: number } }
    ).dictationFixture.level = 84;
  });
  await expect(meter).toHaveAttribute("aria-valuenow", "0");
});

test("the starting strip shows a quiet meter and Stop cancels model startup", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=interaction-question");
  await page.evaluate(() => {
    (
      window as unknown as { dictationFixture: { starting: boolean } }
    ).dictationFixture.starting = true;
  });
  await startRecording(page);
  await expect(page.getByText("Starting mic", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("meter", { name: "Microphone input level", exact: true }),
  ).toHaveAttribute("aria-valuenow", "0");
  await page
    .getByRole("button", { name: "Stop dictation", exact: true })
    .click();
  await expect(
    page.getByRole("group", { name: "Dictation recording", exact: true }),
  ).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { dictationFixture: { calls: string[] } })
            .dictationFixture.calls,
      ),
    )
    .toContain("control_dictation:abort");
});

for (const enabled of [true, false]) {
  test(`spoken punctuation can be ${enabled ? "enabled" : "disabled"} before recording`, async ({
    page,
  }) => {
    await page.goto("/?fixture=interaction-question");
    await page.evaluate(() => {
      (
        window as unknown as { dictationFixture: { initialText: string } }
      ).dictationFixture.initialText = "Ready question mark.";
    });
    await page
      .getByRole("button", { name: "Start offline dictation", exact: true })
      .click();
    const setting = page.getByRole("checkbox", {
      name: "Spoken punctuation",
      exact: true,
    });
    await expect(setting).toBeChecked();
    if (!enabled) await setting.uncheck();
    await page
      .getByRole("button", { name: "Choose model…", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Start recording", exact: true })
      .click();
    await expect(
      page.getByRole("textbox", { name: "Prompt", exact: true }),
    ).toHaveValue(enabled ? "Ready?" : "Ready question mark.");
  });
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
