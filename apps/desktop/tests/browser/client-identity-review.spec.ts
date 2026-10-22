import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

const firstFingerprint = "a1".repeat(32);
const secondFingerprint = "b2".repeat(32);
const palettes = ["light", "colossus", "neutral", "black", "hacker"] as const;

async function installNativeReview(
  page: Page,
  palette: (typeof palettes)[number] = "colossus",
  textSize: "comfortable" | "large" = "comfortable",
  failure: "context" | "finish" | null = null,
) {
  await page.addInitScript(
    ({ palette, textSize, firstFingerprint, secondFingerprint, failure }) => {
      const calls: unknown[] = [];
      Object.assign(window, {
        identityReviewCalls: calls,
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, args });
            if (command === "client_identity_review_context") {
              if (failure === "context")
                throw new Error("PRIVATE_NATIVE_DETAIL");
              return {
                uiToken: "native-one-use-ui-token",
                origin: "https://accounts.example.test:8443",
                candidates: [
                  {
                    label:
                      "Alex Morgan — Example Issuing CA · Client authentication",
                    fingerprintSha256: firstFingerprint,
                  },
                  {
                    label:
                      "Alex Morgan — Research Issuing CA · <script>plain text</script>",
                    fingerprintSha256: secondFingerprint,
                  },
                ],
                appearance: {
                  colorTheme: palette === "light" ? "light" : "dark",
                  darkPalette: palette === "light" ? "colossus" : palette,
                  textSize,
                },
              };
            }
            if (command === "finish_client_identity_review") {
              if (failure === "finish")
                throw new Error("PRIVATE_NATIVE_DETAIL");
              return;
            }
            throw new Error("Unexpected native command");
          },
        },
      });
    },
    { palette, textSize, firstFingerprint, secondFingerprint, failure },
  );
  await page.goto("/?surface=client-identity-review");
}

async function calls(page: Page) {
  return page.evaluate(
    () =>
      (window as unknown as { identityReviewCalls: unknown[] })
        .identityReviewCalls,
  );
}

for (const palette of palettes) {
  for (const compact of [false, true]) {
    test(`certificate review ${palette}, ${compact ? "compact Large" : "comfortable"}`, async ({
      page,
    }) => {
      await page.setViewportSize(
        compact ? { width: 420, height: 360 } : { width: 1000, height: 780 },
      );
      await installNativeReview(
        page,
        palette,
        compact ? "large" : "comfortable",
      );
      const dialog = page.getByRole("dialog", { name: "Choose a certificate" });
      await expect(dialog).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Cancel", exact: true }),
      ).toBeFocused();
      await expect(
        page.getByRole("button", { name: "Use certificate", exact: true }),
      ).toBeDisabled();
      await expect(page.locator("html")).toHaveAttribute(
        "data-theme",
        palette === "light" ? "light" : "dark",
      );
      await expect(page.locator("html")).toHaveAttribute(
        "data-palette",
        palette === "light" ? "colossus" : palette,
      );
      await expect(page.locator("html")).toHaveAttribute(
        "data-text-size",
        compact ? "large" : "comfortable",
      );
      await expect(dialog).toContainText("https://accounts.example.test:8443");
      const second = page.getByRole("radio", { name: /Research Issuing CA/ });
      await second.focus();
      await page.keyboard.press("Space");
      await expect(second).toHaveAttribute("aria-checked", "true");
      await expect(
        page.getByRole("button", { name: "Use certificate", exact: true }),
      ).toBeEnabled();
      await expect(
        page.locator("script", { hasText: "plain text" }),
      ).toHaveCount(0);
      await expect(dialog).toContainText("<script>plain text</script>");
      await expect(
        page.getByRole("button", { name: "Cancel", exact: true }),
      ).toBeInViewport();
      await expect(
        page.getByRole("button", { name: "Use certificate", exact: true }),
      ).toBeInViewport();
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBe(compact ? 420 : 1000);
      expect((await calls(page)).length).toBe(1);
      const scan = await new AxeBuilder({ page }).analyze();
      expect(scan.violations).toEqual([]);
      await page.screenshot({
        path: `output/playwright/client-identity-${palette}-${compact ? "compact-large" : "comfortable"}.png`,
      });
    });
  }
}

test("keyboard choice needs explicit confirmation and sends only native token and fingerprint", async ({
  page,
}) => {
  await installNativeReview(page);
  const first = page.getByRole("radio", { name: /Example Issuing CA/ });
  const second = page.getByRole("radio", { name: /Research Issuing CA/ });
  await first.focus();
  await page.keyboard.press("ArrowDown");
  await expect(second).toBeFocused();
  await expect(second).toHaveAttribute("aria-checked", "true");
  expect((await calls(page)).length).toBe(1);
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Use certificate", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(second).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(
    page.getByRole("button", { name: "Use certificate", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect
    .poll(() => calls(page))
    .toEqual([
      { command: "client_identity_review_context", args: {} },
      {
        command: "finish_client_identity_review",
        args: {
          uiToken: "native-one-use-ui-token",
          fingerprintSha256: secondFingerprint,
        },
      },
    ]);
  await expect(
    page.getByRole("button", { name: "Applying choice…" }),
  ).toBeDisabled();
});

for (const key of ["Enter", "Escape"]) {
  test(`${key} defaults to cancel without selecting any identity`, async ({
    page,
  }) => {
    await installNativeReview(page);
    await expect(
      page.getByRole("button", { name: "Cancel", exact: true }),
    ).toBeFocused();
    await page.keyboard.press(key);
    await expect
      .poll(() => calls(page))
      .toEqual([
        { command: "client_identity_review_context", args: {} },
        {
          command: "finish_client_identity_review",
          args: { uiToken: "native-one-use-ui-token", fingerprintSha256: null },
        },
      ]);
  });
}

test("stale native context exposes no confirmation or private error details", async ({
  page,
}) => {
  await installNativeReview(page, "colossus", "comfortable", "context");
  await expect(page.getByRole("alert")).toContainText("no longer available");
  await expect(
    page.getByRole("button", { name: "Use certificate" }),
  ).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("PRIVATE_NATIVE_DETAIL");
});

test("expired decision stays disabled and does not retry", async ({ page }) => {
  await installNativeReview(page, "colossus", "comfortable", "finish");
  await page.getByRole("radio", { name: /Example Issuing CA/ }).click();
  await page
    .getByRole("button", { name: "Use certificate", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "Check the site for the result",
  );
  await expect(
    page.getByRole("button", { name: "Applying choice…" }),
  ).toBeDisabled();
  await expect(page.locator("body")).not.toContainText("PRIVATE_NATIVE_DETAIL");
  await page.keyboard.press("Escape");
  expect((await calls(page)).length).toBe(2);
});
