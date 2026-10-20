import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";

const config = JSON.parse(
  readFileSync(
    new URL("../../src-tauri/tauri.conf.json", import.meta.url),
    "utf8",
  ),
) as { app: { security: { devCsp: string } } };

test("native development CSP permits Vite startup and keeps event handlers blocked", async ({
  page,
  context,
  browser,
  browserName,
}) => {
  // An intercepted document loses Chromium's loopback address-space metadata.
  // Chrome 142+ requires LNA permission for its genuine Vite HMR socket; this
  // grant applies only to this test's temporary context and loopback origin.
  if (
    browserName === "chromium" &&
    Number.parseInt(browser.version(), 10) >= 142
  ) {
    await context.grantPermissions(["local-network-access"], {
      origin: "http://127.0.0.1:1422",
    });
  }
  const errors: string[] = [];
  let hmrConnected = false;
  page.on("websocket", (socket) => {
    socket.on("framereceived", ({ payload }) => {
      if (payload === '{"type":"connected"}') hmrConnected = true;
    });
  });
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.route("http://127.0.0.1:1422/", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      headers: {
        ...response.headers(),
        "content-security-policy": config.app.security.devCsp,
      },
    });
  });
  const response = await page.goto("/");
  expect(response?.headers()["content-security-policy"]).toBe(
    config.app.security.devCsp,
  );
  await expect(
    page.getByRole("heading", { name: "Connect to Colossus", exact: true }),
  ).toBeVisible();
  expect(
    await page.locator(".mobile-login").evaluate((element) => ({
      width: element.getBoundingClientRect().width,
      viewport: innerWidth,
    })),
  ).toEqual({ width: 390, viewport: 390 });
  await page
    .getByRole("button", { name: "Explore development preview", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Recent work", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Message Colossus", exact: true }),
  ).toBeVisible();
  await expect.poll(() => hmrConnected).toBe(true);
  expect(errors).toEqual([]);
  const inlineHandlerRan = await page.evaluate(() => {
    const probe = document.createElement("button");
    probe.setAttribute("onclick", "this.dataset.executed = 'true'");
    document.body.append(probe);
    probe.click();
    const ran = probe.dataset.executed === "true";
    probe.remove();
    return ran;
  });
  expect(inlineHandlerRan).toBe(false);
});
