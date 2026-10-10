import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

async function open(
  page: import("@playwright/test").Page,
  unavailable = false,
) {
  await page.goto(
    `/?fixture=operations-studio${unavailable ? "&inboxUnavailable=1" : ""}`,
  );
  await page.getByRole("button", { name: "Inboxes", exact: true }).click();
}

async function receipt(page: import("@playwright/test").Page, name: string) {
  await page.getByRole("combobox", { name: "Receipt", exact: true }).click();
  await page.getByRole("option", { name, exact: true }).click();
}

test("inboxes retain receipts, page in order, and display peer text without executing it", async ({
  page,
}) => {
  await open(page);
  const inbox = page.getByRole("region", {
    name: "Agent inboxes",
    exact: true,
  });
  await expect(inbox.locator("article")).toHaveCount(16);
  await expect(inbox).toContainText(
    "<script>Untrusted peer text stays visible as text.</script>",
  );
  await expect(inbox.locator("script")).toHaveCount(0);
  await inbox.getByRole("button", { name: "Load more messages" }).click();
  await expect(inbox.locator("article")).toHaveCount(19);
  await receipt(page, "Undelivered");
  await expect(inbox.locator("article")).toHaveCount(1);
  await expect(inbox).toContainText("Undelivered · interrupted");
  await receipt(page, "All receipts");
  await inbox
    .getByRole("textbox", { name: "Search loaded messages" })
    .fill("fixture-message-1");
  await expect(inbox.locator("article")).toHaveCount(11);
  await inbox.getByRole("textbox", { name: "Search loaded messages" }).fill("");
  const recipient = inbox.getByRole("combobox", { name: "Recipient attempt" });
  await recipient.focus();
  await recipient.press("End");
  await recipient.press("Enter");
  await expect(inbox).toContainText("No messages have been accepted");
  await recipient.focus();
  await expect(recipient).toBeFocused();
  await inbox.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(inbox.locator("article")).toHaveCount(16);
});

test("an unavailable connection is distinguishable from an empty inbox", async ({
  page,
}) => {
  await open(page, true);
  await expect(
    page.getByRole("region", { name: "Agent inboxes" }),
  ).toContainText("This connection does not provide inbox inspection");
  await expect(
    page.getByText("No messages have been accepted in this inbox."),
  ).toHaveCount(0);
});

for (const [theme, palette, width, size] of [
  ["light", "colossus", 1100, "comfortable"],
  ["dark", "colossus", 880, "compact"],
  ["dark", "neutral", 620, "large"],
  ["dark", "hacker", 620, "large"],
] as const) {
  test(`inbox inspection is accessible in ${theme}/${palette}/${size}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await open(page);
    await page.evaluate(
      ({ theme, palette, size }) => {
        document.documentElement.dataset.theme = theme;
        document.documentElement.dataset.palette = palette;
        document.documentElement.dataset.textSize = size;
      },
      { theme, palette, size },
    );
    const inbox = page.getByRole("region", {
      name: "Agent inboxes",
      exact: true,
    });
    await expect(inbox.locator("article")).toHaveCount(16);
    await receipt(page, "Included in a turn");
    expect(
      await inbox.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    const accessibility = await new AxeBuilder({ page })
      .include(".agent-inbox")
      .analyze();
    expect(accessibility.violations).toEqual([]);
    await inbox
      .getByRole("heading", { name: "Agent inboxes" })
      .scrollIntoViewIfNeeded();
    await page.screenshot({
      path: testInfo.outputPath(`inbox-${theme}-${palette}-${size}.png`),
    });
    const message = inbox.locator("article").first();
    await message.getByText("Message details", { exact: true }).click();
    await expect(
      message.getByText("Consuming run", { exact: true }),
    ).toBeVisible();
    await message
      .getByText("Input request hash", { exact: true })
      .scrollIntoViewIfNeeded();
    await expect(
      message.getByText("Input request hash", { exact: true }),
    ).toBeVisible();
    expect(
      await message.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(
        `inbox-details-${theme}-${palette}-${size}.png`,
      ),
    });
  });
}
