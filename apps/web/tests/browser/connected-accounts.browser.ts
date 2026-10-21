import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";

test("credentials can be saved unused and OAuth accounts have their own management page", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { administer: true });
  await page.goto("/settings?project=browser-project");
  const sections = page.getByRole("navigation", { name: "Settings sections" });
  await sections
    .getByRole("button", { name: "Credentials", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Add credential", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Display label", exact: true })
    .fill("Unattached account key");
  await page
    .locator('input[type="password"]')
    .fill("synthetic-unattached-secret");
  await page
    .getByRole("button", { name: "Save credential", exact: true })
    .click();
  await expect(page.getByRole("cell", { name: /Not attached/u })).toBeVisible();
  expect(fixture.credentialWrites[0]!.body).not.toHaveProperty("attachment");
  await sections
    .getByRole("button", { name: "Connected accounts", exact: true })
    .click();
  const chatgpt = page.getByRole("region", {
    name: "ChatGPT / Codex",
    exact: true,
  });
  await expect(chatgpt).toContainText("Server setup required");
  await chatgpt
    .getByRole("button", { name: "View setup requirements", exact: true })
    .click();
  await expect(
    chatgpt.getByRole("link", { name: "OpenAI connection requirements" }),
  ).toHaveAttribute(
    "href",
    "https://developers.openai.com/siwc/token-sharing-open-source",
  );
  const mcp = page.getByRole("region", {
    name: "Personal source",
    exact: true,
  });
  await mcp.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(
    mcp.getByRole("link", { name: "Continue sign-in for Personal source" }),
  ).toHaveAttribute("href", /^https:\/\/identity\.example\/authorize/u);
  expect(fixture.connectionActions).toEqual(["auth"]);
  await page.screenshot({
    path: "output/playwright/connected-accounts.png",
    fullPage: true,
  });
  // The existing chat route still observes the same account after its OAuth callback.
  fixture.authenticatePersonal();
  await expect(
    mcp.getByRole("button", { name: "Reconnect", exact: true }),
  ).toBeVisible({ timeout: 10000 });
  await mcp.getByRole("button", { name: "Disconnect", exact: true }).click();
  expect(fixture.connectionActions).toEqual(["auth", "disconnect"]);
  fixture.verify();
});
