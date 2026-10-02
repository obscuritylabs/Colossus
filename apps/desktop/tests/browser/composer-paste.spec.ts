import { expect, test } from "@playwright/test";

async function pasteText(
  prompt: import("@playwright/test").Locator,
  text: string,
  start: number,
  end: number,
) {
  await prompt.evaluate(
    (element, paste) => {
      const textarea = element as HTMLTextAreaElement;
      textarea.focus();
      textarea.setSelectionRange(paste.start, paste.end);
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", paste.text);
      textarea.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData,
        }),
      );
    },
    { text, start, end },
  );
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 880, height: 640 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page
    .getByRole("dialog", { name: "Workspace navigation" })
    .getByRole("button", { name: "New thread in Colossus" })
    .click();
});

test("large pasted text stays compact and the full text is sent", async ({
  page,
}) => {
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  const pasted = `# Heading\r\n${"  - detail 👩‍💻 界\r\n".repeat(90)}`;
  const normalized = pasted.replace(/\r\n/g, "\n");
  const chars = [...normalized].length;
  await prompt.fill("Before  after");
  await pasteText(prompt, pasted, 7, 7);
  await expect(prompt).toHaveValue(
    `Before [Pasted Content ${chars} chars] after`,
  );
  await expect(page.locator(".composer-paste-summary")).toContainText(
    "1 large paste condensed. Full text is included when sent.",
  );
  expect((await prompt.boundingBox())!.height).toBeLessThan(100);

  await prompt.press("Enter");
  await expect(page.locator(".message-user .message-body")).toHaveText(
    `Before ${normalized} after`,
  );
  await expect(prompt).toHaveValue("");
  await expect(page.locator(".composer-paste-summary")).toHaveCount(0);
});

test("the byte limit uses hidden text and removing a marker drops its payload", async ({
  page,
}) => {
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await pasteText(prompt, "x".repeat(65_537), 0, 0);
  await expect(prompt).toHaveValue("[Pasted Content 65537 chars]");
  await expect(page.locator(".counter-over-limit")).toContainText(
    "65,537 / 65,536 bytes",
  );
  await expect(
    page.getByRole("button", { name: "Send prompt" }),
  ).toBeDisabled();
  await prompt.fill("Short message");
  await expect(page.locator(".composer-paste-summary")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Send prompt" })).toBeEnabled();
  await prompt.press("Enter");
  await expect(page.locator(".message-user .message-body")).toHaveText(
    "Short message",
  );
});

test("backspacing the marker edge detaches the hidden paste", async ({
  page,
}) => {
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("]");
  await pasteText(prompt, "z".repeat(1_001), 0, 0);
  const marker = "[Pasted Content 1001 chars]";
  await expect(prompt).toHaveValue(`${marker}]`);
  await prompt.evaluate((element, cursor) => {
    (element as HTMLTextAreaElement).setSelectionRange(cursor, cursor);
  }, marker.length);
  await prompt.press("Backspace");
  await expect(prompt).toHaveValue(`${marker.slice(0, -1)}]`);
  await expect(page.locator(".composer-paste-summary")).toHaveCount(0);
});

test("queued follow-ups contain the full paste", async ({ page }) => {
  await page.goto("/?fixture=interaction-question");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  const pasted = "Read this follow-up carefully.\n".repeat(60);
  await pasteText(prompt, pasted, 0, 0);
  await expect(prompt).toHaveValue(/\[Pasted Content \d+ chars\]/);
  await page.getByRole("button", { name: "Add message to Next up" }).click();
  await expect(page.locator(".next-up-copy p")).toHaveText(pasted.trim());
  await expect(prompt).toHaveValue("");
});
