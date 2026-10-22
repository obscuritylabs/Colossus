import { expect, test, type Locator } from "@playwright/test";
import { openConversationList } from "./appearance";
import { browserFixture } from "./fixture";

const longTitle =
  "Review the shared workspace layout and preserve this complete conversation title when the sidebar is narrow";
const appearances = [
  { id: "dark", colorTheme: "dark", darkPalette: "colossus" },
  { id: "light", colorTheme: "light", darkPalette: "colossus" },
  { id: "neutral", colorTheme: "dark", darkPalette: "neutral" },
  { id: "black", colorTheme: "dark", darkPalette: "black" },
  { id: "hacker", colorTheme: "dark", darkPalette: "hacker" },
] as const;

async function rowGeometry(row: Locator, action: Locator) {
  const [title, button, rowBounds] = await Promise.all([
    row
      .locator(".shared-workspace-thread-copy > strong")
      .evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return {
          height: bounds.height,
          right: bounds.right,
          lineHeight: Number.parseFloat(style.lineHeight),
          clientWidth: element.clientWidth,
          scrollWidth: element.scrollWidth,
          textOverflow: style.textOverflow,
        };
      }),
    action.boundingBox(),
    row.boundingBox(),
  ]);
  return { title, button: button!, row: rowBounds! };
}

for (const appearance of appearances) {
  test(`single-line ${appearance.id} chat titles retain their full accessible context at every text size`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      plan: true,
      branches: true,
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((value) => {
      const key = "colossus.web.appearance.v1";
      if (!localStorage.getItem(key))
        localStorage.setItem(
          key,
          JSON.stringify({ ...value, textSize: "compact" }),
        );
    }, appearance);
    await page.route(
      "**/api/projects/browser-project/chat/threads?**",
      async (route) => {
        await route.fulfill({
          json: {
            threads: [
              {
                thread_id: "browser-thread",
                project_id: "browser-project",
                owner_user_id: "browser-user",
                node_id: fixture.nodeId,
                title: longTitle,
                created_at: "2026-10-16T10:00:00Z",
                updated_at: "2026-10-16T10:00:00Z",
                revision: 1,
                archived: false,
                session_id: "browser-session",
                sync_status: "current",
                source: "cloud",
                can_continue: true,
              },
            ],
            next_cursor: null,
          },
        });
      },
    );
    await page.goto("/projects/browser-project/chat/browser-thread");
    for (const textSize of ["compact", "comfortable", "large"]) {
      if (textSize !== "compact") {
        await page.evaluate((size) => {
          const key = "colossus.web.appearance.v1";
          localStorage.setItem(
            key,
            JSON.stringify({
              ...JSON.parse(localStorage.getItem(key)!),
              textSize: size,
            }),
          );
        }, textSize);
        await page.reload();
      }
      await expect(page.locator("html")).toHaveAttribute(
        "data-text-size",
        textSize,
      );
      await expect(page.locator("html")).toHaveAttribute(
        "data-theme",
        appearance.colorTheme,
      );
      await expect(page.locator("html")).toHaveAttribute(
        "data-palette",
        appearance.darkPalette,
      );
      await openConversationList(page);
      const history = page.getByRole("navigation", {
        name: "Your chat history",
        exact: true,
      });
      const row = history.getByRole("link", { name: longTitle, exact: true });
      const action = history.getByRole("button", {
        name: `Chat actions for ${longTitle}`,
        exact: true,
      });
      await expect(row).toHaveAttribute("aria-current", "page");
      await expect(row).toHaveAccessibleDescription(/Updated /u);
      await expect(row).toHaveAttribute("title", new RegExp(longTitle, "u"));
      await expect(row).toHaveAttribute("title", /Updated /u);
      const geometry = await rowGeometry(row, action);
      expect(geometry.row.height).toBeLessThanOrEqual(44);
      expect(geometry.title.height).toBeLessThanOrEqual(
        geometry.title.lineHeight + 1,
      );
      expect(geometry.title.scrollWidth).toBeGreaterThan(
        geometry.title.clientWidth,
      );
      expect(geometry.title.textOverflow).toBe("ellipsis");
      expect(geometry.title.right).toBeLessThanOrEqual(geometry.button.x);
      expect(geometry.button.y).toBeGreaterThanOrEqual(geometry.row.y);
      expect(geometry.button.y + geometry.button.height).toBeLessThanOrEqual(
        geometry.row.y + geometry.row.height + 1,
      );
      expect(
        await action.evaluate((element) => element.closest("a")),
      ).toBeNull();
    }
    const action = page.getByRole("button", {
      name: `Chat actions for ${longTitle}`,
      exact: true,
    });
    await action.focus();
    await action.press("ArrowDown");
    const rename = page.getByRole("menuitem", { name: "Rename", exact: true });
    await expect(rename).toBeFocused();
    await expect(page.getByRole("menu")).toContainText(longTitle);
    await rename.press("ArrowDown");
    await expect(
      page.getByRole("menuitem", { name: "Fork chat", exact: true }),
    ).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(action).toBeFocused();
    expect(fixture.threadMutations).toHaveLength(0);
    expect(fixture.branchSubmissions).toHaveLength(0);
    await page.screenshot({
      path: `output/playwright/chat-history-compact-${appearance.id}-large.png`,
    });
    fixture.verify();
  });
}
