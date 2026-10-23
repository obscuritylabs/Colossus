import { expect, test } from "@playwright/test";
import type { Classification } from "../../src/control-api";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

test("routed save notifications stay above a wrapping bottom classification strip", async ({
  page,
}, testInfo) => {
  const fixture = await browserFixture(page, { administer: true });
  const classification: Classification = {
    enabled: true,
    text: [..."INTERNAL · CONTROL PLANE REVIEW 🔒 ".repeat(6)]
      .slice(0, 160)
      .join(""),
    tone: "warning",
    position: "top_and_bottom",
  };
  await page.route("**/api/settings", (route) =>
    route.fulfill({ json: { classification } }),
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await selectAppearance(page, appearanceCases[1]);
  await page.goto("/projects/browser-project/settings/providers/company/edit");
  await page
    .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
    .fill("Reviewed provider");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(page).toHaveURL(
    /\/projects\/browser-project\/settings\/providers$/u,
  );
  await expect(
    page.getByRole("region", { name: "Settings notifications", exact: true }),
  ).toContainText("Project configuration saved.");

  const geometry = () =>
    page.evaluate(() => {
      const frame = document.querySelector<HTMLElement>(
        ".control-plane-frame",
      )!;
      const strip = frame.querySelector<HTMLElement>(
        ":scope > .control-classification:last-child",
      );
      const toast = frame.querySelector<HTMLElement>(
        '.toast-region[data-placement="bottom"] .app-toast',
      );
      const stripBounds = strip?.getBoundingClientRect();
      const toastBounds = toast?.getBoundingClientRect();
      const measured = Number.parseFloat(
        getComputedStyle(frame).getPropertyValue(
          "--control-plane-bottom-classification-height",
        ),
      );
      return {
        height: stripBounds?.height ?? 0,
        clear: Boolean(
          stripBounds &&
          toastBounds &&
          toastBounds.bottom <= stripBounds.top - 15,
        ),
        measured: Boolean(
          stripBounds && Math.abs(measured - stripBounds.height) < 1,
        ),
      };
    });
  await expect.poll(async () => (await geometry()).clear).toBe(true);
  const wideHeight = (await geometry()).height;
  await page.setViewportSize({ width: 390, height: 844 });
  await expect
    .poll(async () => {
      const narrow = await geometry();
      return narrow.clear && narrow.measured && narrow.height > wideHeight;
    })
    .toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("large-mobile-toast-above-classification.png"),
    fullPage: true,
  });
  expect(fixture.settingsSubmissions).toHaveLength(1);

  classification.enabled = false;
  await page.reload();
  await expect(
    page.locator(".control-plane-frame > .control-classification"),
  ).toHaveCount(0);
  await expect(page.locator(".control-plane-frame")).toHaveCSS(
    "--control-plane-bottom-classification-height",
    "0px",
  );
  fixture.verify();
});
