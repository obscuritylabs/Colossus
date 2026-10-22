import { expect, test, type Page } from "@playwright/test";
import type { Thread } from "../../src/api";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

const project = "browser-project";
const parentTitle = "Browser model conversation";
const recentTitle = "Review the shared conversation layout";
const olderTitle = "Older architecture discussion";
const parentUrl = `/projects/${project}/chat/browser-thread`;

/** The production app reads only isolated, account-owned synthetic records here. */
async function pinFixture(page: Page) {
  const fixture = await browserFixture(page, { managedChat: true });
  const rows: Thread[] = [
    ["browser-thread", parentTitle, "2026-10-21T10:00:00Z"],
    ["pin-recent", recentTitle, "2026-10-20T10:00:00Z"],
    ["pin-older", olderTitle, "2026-09-01T10:00:00Z"],
  ].map(([id, title, updated]) => ({
    thread_id: id!,
    project_id: project,
    owner_user_id: "browser-user",
    node_id: fixture.nodeId,
    session_id: `${id}-session`,
    title: title!,
    revision: 1,
    created_at: updated!,
    updated_at: updated!,
    archived: false,
    source: "cloud",
    sync_status: "current",
    can_continue: true,
  }));
  const mutations: { thread_id: string; archived?: boolean }[] = [];
  const detailReads: string[] = [];
  await page.route(`**/api/projects/${project}/**`, async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (
      request.method() === "GET" &&
      url.pathname === `/api/projects/${project}/chat/threads`
    ) {
      const archived = url.searchParams.get("archived") === "true";
      const query = (url.searchParams.get("query") ?? "").toLowerCase();
      const matching = rows.filter(
        (row) =>
          row.archived === archived && row.title.toLowerCase().includes(query),
      );
      const start = url.searchParams.get("after") === "older-page" ? 2 : 0;
      return route.fulfill({
        json: {
          threads: query ? matching : matching.slice(start, start + 2),
          next_cursor:
            !query && start === 0 && matching.length > 2 ? "older-page" : null,
        },
      });
    }
    const [id, suffix] = url.pathname
      .slice(`/api/projects/${project}/threads/`.length)
      .split("/");
    const row = rows.find((candidate) => candidate.thread_id === id);
    if (
      row &&
      request.method() === "GET" &&
      (!suffix || suffix === "metadata")
    ) {
      detailReads.push(suffix === "metadata" ? `${id}/metadata` : id!);
      return route.fulfill({
        json:
          suffix === "metadata"
            ? { thread: row }
            : {
                thread: row,
                messages: [
                  {
                    message_id: `${id}-message`,
                    role: "user",
                    text: "Retained personal chat question",
                    created_at: row.created_at,
                  },
                ],
                tasks: [],
              },
      });
    }
    if (row && !suffix && request.method() === "PATCH") {
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON();
      expect(body.revision).toBe(row.revision);
      mutations.push({ thread_id: id!, ...body });
      Object.assign(row, body, { revision: row.revision + 1 });
      return route.fulfill({ json: { thread: row } });
    }
    return route.fallback();
  });
  return { ...fixture, rows, mutations, detailReads };
}

async function openHistory(page: Page) {
  const trigger = page.getByRole("button", {
    name: "Toggle conversation list",
    exact: true,
  });
  await expect(
    page.getByRole("heading", { name: parentTitle, exact: true }),
  ).toBeVisible();
  await expect(trigger).toHaveAttribute("aria-expanded", /^(true|false)$/u);
  if ((await trigger.getAttribute("aria-expanded")) === "false")
    await trigger.click();
  await expect(
    page.getByRole("region", { name: "Your chats", exact: true }),
  ).toBeVisible();
}

function history(page: Page) {
  return page.getByRole("navigation", {
    name: "Your chat history",
    exact: true,
  });
}

function pinned(page: Page) {
  return history(page).getByRole("region", { name: "Pinned", exact: true });
}

function actions(page: Page, title: string) {
  return history(page).getByRole("button", {
    name: `Chat actions for ${title}`,
    exact: true,
  });
}

async function togglePin(page: Page, title: string, unpin = false) {
  const trigger = actions(page, title);
  await trigger.focus();
  await trigger.press("Enter");
  const item = page.getByRole("menuitemcheckbox", {
    name: `${unpin ? "Unpin" : "Pin"} ${title}`,
    exact: true,
  });
  await expect(item).toHaveAttribute("aria-checked", String(unpin));
  await item.focus();
  await item.press("Enter");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(actions(page, title)).toBeFocused();
}

async function pinnedTitles(page: Page) {
  return pinned(page)
    .getByRole("link")
    .evaluateAll((links) =>
      links.map((link) => link.getAttribute("aria-label")),
    );
}

test("keyboard pinning relocates compact rows without navigation or a server mutation", async ({
  page,
}) => {
  const fixture = await pinFixture(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentUrl);
  await openHistory(page);
  const selected = history(page).getByRole("link", {
    name: parentTitle,
    exact: true,
  });
  await expect(selected).toHaveAttribute("aria-current", "page");
  await togglePin(page, parentTitle);
  await expect(pinned(page)).toBeVisible();
  await expect(pinned(page).getByRole("link")).toHaveCount(1);
  await expect(selected).toHaveAccessibleDescription(/Pinned/u);
  await expect(page).toHaveURL(parentUrl);
  await togglePin(page, recentTitle);
  expect(await pinnedTitles(page)).toEqual([recentTitle, parentTitle]);
  await expect(history(page).getByRole("link")).toHaveCount(2);
  await page.reload();
  await openHistory(page);
  await expect(pinned(page).getByRole("link")).toHaveCount(2);
  expect(await pinnedTitles(page)).toEqual([recentTitle, parentTitle]);
  await togglePin(page, recentTitle, true);
  expect(await pinnedTitles(page)).toEqual([parentTitle]);
  await expect(
    history(page).getByRole("link", { name: recentTitle, exact: true }),
  ).toHaveCount(1);
  await togglePin(page, parentTitle, true);
  await expect(pinned(page)).toHaveCount(0);
  await expect(history(page).getByRole("link")).toHaveCount(2);
  expect(fixture.mutations).toEqual([]);
  expect(fixture.submissions).toEqual([]);
  fixture.verify();
});

test("an older pinned chat is rehydrated beyond the current history page and follows search", async ({
  page,
}) => {
  const fixture = await pinFixture(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentUrl);
  await openHistory(page);
  await page.getByRole("button", { name: "Older chats", exact: true }).click();
  await expect(
    history(page).getByRole("link", { name: olderTitle, exact: true }),
  ).toBeVisible();
  await togglePin(page, olderTitle);
  await page.getByRole("button", { name: "Newer chats", exact: true }).click();
  await expect(
    pinned(page).getByRole("link", { name: olderTitle }),
  ).toBeVisible();
  await expect(history(page).getByRole("link")).toHaveCount(3);
  fixture.detailReads.length = 0;
  await page.reload();
  await openHistory(page);
  await expect(
    pinned(page).getByRole("link", { name: olderTitle }),
  ).toBeVisible();
  await expect
    .poll(() => fixture.detailReads.includes("pin-older/metadata"))
    .toBe(true);
  const search = page.getByRole("searchbox", { name: "Search your chats" });
  await search.fill("architecture");
  await expect(history(page).getByRole("link")).toHaveCount(1);
  await expect(
    pinned(page).getByRole("link", { name: olderTitle }),
  ).toBeVisible();
  await search.fill("shared conversation");
  await expect(pinned(page)).toHaveCount(0);
  await expect(
    history(page).getByRole("link", { name: recentTitle, exact: true }),
  ).toBeVisible();
  await search.clear();
  await expect(
    pinned(page).getByRole("link", { name: olderTitle }),
  ).toBeVisible();
  await expect(history(page).getByRole("link")).toHaveCount(3);
  expect(fixture.mutations).toEqual([]);
  fixture.verify();
});

test("archiving hides an open pin while Restore preserves it, and archived chats can be unpinned", async ({
  page,
}) => {
  const fixture = await pinFixture(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentUrl);
  await openHistory(page);
  await togglePin(page, parentTitle);
  await actions(page, parentTitle).click();
  await page.getByRole("menuitem", { name: "Archive", exact: true }).click();
  await expect(
    history(page).getByRole("link", { name: parentTitle }),
  ).toHaveCount(0);
  await expect(pinned(page)).toHaveCount(0);
  await page.getByRole("radio", { name: "Archived", exact: true }).click();
  await expect(
    history(page).getByRole("link", { name: parentTitle }),
  ).toBeVisible();
  await actions(page, parentTitle).click();
  await page.getByRole("menuitem", { name: "Restore", exact: true }).click();
  await page.getByRole("radio", { name: "Open", exact: true }).click();
  await expect(
    pinned(page).getByRole("link", { name: parentTitle }),
  ).toBeVisible();
  await actions(page, parentTitle).click();
  await page.getByRole("menuitem", { name: "Archive", exact: true }).click();
  await page.getByRole("radio", { name: "Archived", exact: true }).click();
  await togglePin(page, parentTitle, true);
  await actions(page, parentTitle).click();
  await page.getByRole("menuitem", { name: "Restore", exact: true }).click();
  await page.getByRole("radio", { name: "Open", exact: true }).click();
  await expect(
    history(page).getByRole("link", { name: parentTitle }),
  ).toBeVisible();
  await expect(pinned(page)).toHaveCount(0);
  expect(fixture.mutations.map((row) => row.archived)).toEqual([
    true,
    false,
    true,
    false,
  ]);
  fixture.verify();
});

test("Pinned collapses with the keyboard, remembers its state, and reveals search matches without losing the chat draft", async ({
  page,
}) => {
  const fixture = await pinFixture(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentUrl);
  await openHistory(page);
  const composer = page
    .locator(".shared-inspector-main > .thread-detail")
    .getByRole("textbox", { name: "Message the agent", exact: true });
  await composer.fill("Keep this selected conversation draft");
  await togglePin(page, parentTitle);
  const heading = pinned(page).getByRole("button", {
    name: "Pinned",
    exact: true,
  });
  const selected = pinned(page).getByRole("link", {
    name: parentTitle,
    exact: true,
  });
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  const controlledId = await heading.getAttribute("aria-controls");
  expect(controlledId).toBeTruthy();
  const controlledRows = page.locator(`[id=${JSON.stringify(controlledId)}]`);
  await expect(controlledRows).toHaveCount(1);
  await heading.focus();
  await heading.press("Enter");
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await expect(controlledRows).toBeHidden();
  await expect(heading).toBeFocused();
  await expect(composer).toHaveValue("Keep this selected conversation draft");
  await expect(page).toHaveURL(parentUrl);
  await heading.press("Space");
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  await expect(controlledRows).toBeVisible();
  await expect(selected).toHaveAttribute("aria-current", "page");
  await expect(composer).toHaveValue("Keep this selected conversation draft");
  await heading.press("Space");
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await page.reload();
  await openHistory(page);
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await expect(selected).toBeHidden();
  await expect(page).toHaveURL(parentUrl);
  await composer.fill("Keep this draft after reload");
  const search = page.getByRole("searchbox", { name: "Search your chats" });
  await search.fill("Browser model");
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  await expect(selected).toBeVisible();
  await heading.focus();
  await heading.press("Enter");
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await expect(selected).toBeHidden();
  await search.fill("Browser model conversation");
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  await expect(selected).toBeVisible();
  await search.clear();
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await expect(selected).toBeHidden();
  await expect(composer).toHaveValue("Keep this draft after reload");
  await togglePin(page, recentTitle);
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  await expect(pinned(page).getByRole("link")).toHaveCount(2);
  await expect(selected).toHaveAttribute("aria-current", "page");
  await expect(composer).toHaveValue("Keep this draft after reload");
  expect(fixture.mutations).toEqual([]);
  expect(fixture.submissions).toEqual([]);
  fixture.verify();
});

test("cross-tab collapse closes a pinned row menu and preserves unrelated composer focus", async ({
  page,
}) => {
  const fixture = await pinFixture(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(parentUrl);
  await openHistory(page);
  await togglePin(page, parentTitle);
  const composer = page
    .locator(".shared-inspector-main > .thread-detail")
    .getByRole("textbox", { name: "Message the agent", exact: true });
  await composer.fill("Keep typing while another tab changes the sidebar");
  const heading = pinned(page).getByRole("button", {
    name: "Pinned",
    exact: true,
  });
  async function collapseFromOtherTab(collapsed: boolean) {
    await page.evaluate(
      ({ account, project, collapsed }) => {
        const key =
          "colossus.web.chat-pins-collapsed:v1:" +
          JSON.stringify([account, project]);
        const oldValue = localStorage.getItem(key);
        const newValue = JSON.stringify(collapsed);
        localStorage.setItem(key, newValue);
        window.dispatchEvent(
          new StorageEvent("storage", {
            key,
            oldValue,
            newValue,
            storageArea: localStorage,
            url: location.href,
          }),
        );
      },
      { account: "browser-user", project, collapsed },
    );
  }
  await actions(page, parentTitle).focus();
  await actions(page, parentTitle).press("Enter");
  const unpin = page.getByRole("menuitemcheckbox", {
    name: `Unpin ${parentTitle}`,
    exact: true,
  });
  await unpin.focus();
  await expect(unpin).toBeFocused();
  await collapseFromOtherTab(true);
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(heading).toBeFocused();
  await expect(composer).toHaveValue(
    "Keep typing while another tab changes the sidebar",
  );
  await expect(page).toHaveURL(parentUrl);
  await heading.press("Enter");
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  await expect(actions(page, parentTitle)).toBeVisible();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await composer.focus();
  await collapseFromOtherTab(true);
  await expect(heading).toHaveAttribute("aria-expanded", "false");
  await expect(composer).toBeFocused();
  await collapseFromOtherTab(false);
  await expect(heading).toHaveAttribute("aria-expanded", "true");
  await expect(composer).toBeFocused();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(composer).toHaveValue(
    "Keep typing while another tab changes the sidebar",
  );
  expect(fixture.mutations).toEqual([]);
  expect(fixture.submissions).toEqual([]);
  fixture.verify();
});

test.describe("touch pins", () => {
  test.use({ hasTouch: true });
  for (const appearance of appearanceCases.filter(
    (item) => item.id !== "neutral",
  )) {
    test(`pinned rows and menus retain ${appearance.label} Large readability on desktop and mobile`, async ({
      page,
    }) => {
      const fixture = await pinFixture(page);
      await page.setViewportSize({ width: 1440, height: 900 });
      await selectAppearance(page, appearance);
      await page.goto(parentUrl);
      await openHistory(page);
      await togglePin(page, recentTitle);
      for (const width of [1440, 390]) {
        await page.setViewportSize({ width, height: 900 });
        await openHistory(page);
        const row = pinned(page).getByRole("link", {
          name: recentTitle,
          exact: true,
        });
        const trigger = actions(page, recentTitle);
        await expect(row).toBeVisible();
        await expect(row).toHaveAccessibleDescription(/Pinned/u);
        const [rowBounds, triggerBounds] = await Promise.all([
          row.boundingBox(),
          trigger.boundingBox(),
        ]);
        expect(rowBounds!.height).toBeGreaterThanOrEqual(44);
        expect(triggerBounds!.height).toBeGreaterThanOrEqual(44);
        expect(triggerBounds!.width).toBeGreaterThanOrEqual(44);
        await trigger.tap();
        const unpin = page.getByRole("menuitemcheckbox", {
          name: `Unpin ${recentTitle}`,
          exact: true,
        });
        await expect(unpin).toBeChecked();
        await expect(unpin).toBeInViewport({ ratio: 1 });
        expect((await unpin.boundingBox())!.height).toBeGreaterThanOrEqual(44);
        await page.screenshot({
          path: `output/playwright/chat-pins-${appearance.id}-large-${width}.png`,
        });
        await page.keyboard.press("Escape");
        await expect(trigger).toBeFocused();
        await expect(pinned(page)).toBeVisible();
        const heading = pinned(page).getByRole("button", {
          name: "Pinned",
          exact: true,
        });
        await heading.tap();
        await expect(heading).toHaveAttribute("aria-expanded", "false");
        await expect(row).toBeHidden();
        expect((await heading.boundingBox())!.height).toBeGreaterThanOrEqual(
          44,
        );
        await page.screenshot({
          path: `output/playwright/chat-pins-collapsed-${appearance.id}-large-${width}.png`,
        });
        await heading.tap();
        await expect(heading).toHaveAttribute("aria-expanded", "true");
        await expect(row).toBeVisible();
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
      }
      expect(fixture.mutations).toEqual([]);
      fixture.verify();
    });
  }
});
