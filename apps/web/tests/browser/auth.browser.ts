import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";

test("a blocked logout stays visible and can be retried with the keyboard", async ({
  page,
}) => {
  const fixture = await browserFixture(page);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.web.appearance.v1",
      JSON.stringify({
        colorTheme: "dark",
        darkPalette: "neutral",
        textSize: "large",
      }),
    ),
  );
  let attempts = 0;
  await page.route("**/auth/logout", async (route) => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-colossus-csrf"]).toBe("1");
    attempts++;
    return attempts === 1
      ? route.fulfill({
          status: 200,
          contentType: "text/html",
          body: "<html><body>Session refresh required</body></html>",
        })
      : route.fulfill({ status: 204 });
  });
  await page.goto("/fleet?project=browser-project");
  const signOut = page.getByRole("button", { name: "Sign out", exact: true });
  await expect(signOut).toBeVisible();
  await signOut.focus();
  await page.keyboard.press("Enter");
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("Could not sign out");
  await expect(alert).toContainText("Reload this page");
  await expect(signOut).toBeEnabled();
  await expect(
    page.getByRole("heading", { name: "Sign in to your Control Plane" }),
  ).toBeHidden();

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(alert).toBeInViewport();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  const retry = alert.getByRole("button", { name: "Retry sign out" });
  await retry.focus();
  await expect(retry).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Sign in to your Control Plane" }),
  ).toBeVisible();
  await expect(alert).toBeHidden();
  expect(attempts).toBe(2);
  fixture.verify();
});
