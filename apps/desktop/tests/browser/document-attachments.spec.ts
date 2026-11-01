import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { installDesktopExtrasFixture } from "./support/desktop-extras";

for (const [theme, palette, textSize, width] of [
  ["light", "colossus", "large", 700],
  ["dark", "neutral", "large", 700],
  ["dark", "hacker", "large", 700],
  ["dark", "colossus", "compact", 1280],
] as const) {
  test(`document inputs remain explicit and keyboard accessible (${theme}/${palette}/${textSize})`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 990 });
    await page.addInitScript(
      ({ theme, palette, textSize }) => {
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({
            colorTheme: theme,
            darkPalette: palette,
            textSize,
          }),
        );
      },
      { theme, palette, textSize },
    );
    await page.goto("/?fixture=operations-studio");
    if (width === 700)
      await page.getByRole("button", { name: "Open work navigation" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Global", exact: true }).click();
    await page.getByRole("button", { name: "Models", exact: true }).click();
    await page
      .getByRole("button", { name: "Edit primary", exact: true })
      .click();
    const documents = page.getByRole("slider", {
      name: "Document inputs",
      exact: true,
    });
    await expect(documents).toHaveAttribute("aria-valuetext", "Off");
    await page
      .getByRole("group", { name: "Document inputs", exact: true })
      .screenshot({
        path: testInfo.outputPath(
          `document-off-${theme}-${palette}-${textSize}.png`,
        ),
      });
    await documents.focus();
    await documents.press("End");
    await expect(documents).toHaveAttribute("aria-valuetext", "On");
    await page
      .getByRole("group", { name: "Document inputs", exact: true })
      .screenshot({
        path: testInfo.outputPath(
          `document-on-${theme}-${palette}-${textSize}.png`,
        ),
      });
    await documents.press("ArrowLeft");
    await expect(documents).toHaveAttribute("aria-valuetext", "Auto");
    await expect(page.locator(".model-editor")).toContainText(
      "provider-supported PDF and Office attachments",
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    const audit = await new AxeBuilder({ page })
      .include(".model-editor")
      .analyze();
    expect(audit.violations).toEqual([]);
    await page
      .getByRole("group", { name: "Document inputs", exact: true })
      .screenshot({
        path: testInfo.outputPath(
          `document-control-${theme}-${palette}-${textSize}.png`,
        ),
      });
    await page.screenshot({
      path: testInfo.outputPath(
        `document-settings-${theme}-${palette}-${textSize}.png`,
      ),
    });
    await documents.press("Home");
    await expect(documents).toHaveAttribute("aria-valuetext", "Off");
  });
}

test("document chips retain their draft through model preferences and native admission errors", async ({
  page,
}, testInfo) => {
  await installDesktopExtrasFixture(page);
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
    const originalInvoke = host.__TAURI_INTERNALS__.invoke;
    let choices = 0;
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command !== "choose_run_attachment")
        return originalInvoke(command, args);
      choices += 1;
      if (choices > 1)
        throw {
          code: "invalid_argument",
          message: "The request is invalid.",
          retryable: false,
          outcomeUnknown: false,
          violations: [
            {
              field: "attachment",
              description:
                "This model does not advertise support for this document format.",
            },
          ],
        };
      return {
        artifactId: "fixture-owned-pdf",
        fileName: "proposal.pdf",
        mediaType: "application/pdf",
        sizeBytes: 128,
        sha256: "a".repeat(64),
        purpose: "run_input",
        state: "available",
        createdAt: "2026-11-01T00:00:00Z",
      };
    };
  });
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Review this attached proposal.");
  const before = await page.locator('article[data-role="user"]').count();
  await page
    .getByRole("button", { name: "Attach a file", exact: true })
    .click();
  const attachment = page.getByRole("button", {
    name: "Remove proposal.pdf",
    exact: true,
  });
  await expect(attachment).toBeVisible();
  await page
    .getByRole("button", { name: /^Choose model and reasoning:/ })
    .click();
  await page
    .getByRole("listbox", { name: "Models", exact: true })
    .getByRole("option")
    .last()
    .click();
  await expect(attachment).toBeVisible();
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", { name: "Attach a file", exact: true })
    .click();
  await expect(
    page.getByText(
      "This model does not advertise support for this document format.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(attachment).toBeVisible();
  await expect(prompt).toHaveValue("Review this attached proposal.");
  expect(await page.locator('article[data-role="user"]').count()).toBe(before);
  await page.locator(".work-composer").screenshot({
    path: testInfo.outputPath("document-admission-error-keeps-draft.png"),
  });
  await attachment.click();
  await expect(attachment).toHaveCount(0);
});
