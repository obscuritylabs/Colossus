import { expect, type Page } from "@playwright/test";

export const appearanceCases = [
  { id: "dark", label: "dark", theme: "dark", palette: "colossus" },
  { id: "light", label: "light", theme: "light", palette: "colossus" },
  { id: "neutral", label: "neutral dark", theme: "dark", palette: "neutral" },
  { id: "hacker", label: "Hacker dark", theme: "dark", palette: "hacker" },
] as const;

export async function selectAppearance(
  page: Page,
  appearance: (typeof appearanceCases)[number],
) {
  await page.goto("/settings?project=browser-project");
  await page.locator("#appearance-text-size").click();
  await page.getByRole("option", { name: "Large", exact: true }).click();
  await page.locator("#appearance-color-theme").click();
  await page
    .getByRole("option", {
      name: appearance.theme === "dark" ? "Dark" : "Light",
      exact: true,
    })
    .click();
  if (appearance.palette !== "colossus") {
    await page.locator("#appearance-dark-palette").click();
    await page
      .getByRole("option", {
        name:
          appearance.palette === "neutral"
            ? "Neutral dark (Dark+)"
            : "Hacker (TUI)",
        exact: true,
      })
      .click();
  }
  await expect(page.locator("html")).toHaveAttribute("data-text-size", "large");
  await expect(page.locator("html")).toHaveAttribute(
    "data-theme",
    appearance.theme,
  );
  await expect(page.locator("html")).toHaveAttribute(
    "data-palette",
    appearance.palette,
  );
}

export async function openAppSidebar(page: Page) {
  const chat = page.getByRole("link", { name: "Chat", exact: true });
  if (!(await chat.isVisible()))
    await page
      .getByRole("button", { name: "Toggle app sidebar", exact: true })
      .click();
  await expect(chat).toBeVisible();
}

export async function openConversationList(page: Page) {
  const history = page.getByRole("region", {
    name: "Your chats",
    exact: true,
  });
  if (!(await history.isVisible()))
    await page
      .getByRole("button", { name: "Toggle conversation list", exact: true })
      .click();
  await expect(history).toBeVisible();
}
