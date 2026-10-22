import { expect, test, type Page } from "@playwright/test";
import type {
  Announcement,
  AnnouncementContent,
} from "../../src/announcements";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

interface SyntheticInstance {
  records: Map<string, Announcement>;
  receipts: Map<
    string,
    { revision: number; read: boolean; dismissed: boolean }
  >;
  writes: string[];
}
const instance = (): SyntheticInstance => ({
  records: new Map(),
  receipts: new Map(),
  writes: [],
});
const message =
  "## What's new\n\n**Shared updates** across the instance.\n\n```ts\nconst release = 'ready';\n```\n\n[Read the guide](https://example.com/guide)";

/** All writes stay in this synthetic instance, never a running Control Plane. */
async function announcementFixture(
  page: Page,
  state: SyntheticInstance,
  user = "operator",
) {
  await page.route("**/api/announcements**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.method() === "GET") {
      return route.fulfill({
        json: {
          active_banner:
            [...state.records.values()]
              .filter(
                (row) =>
                  row.status === "published" &&
                  row.banner &&
                  (!row.expires_at ||
                    Date.parse(row.expires_at) > Date.now()) &&
                  !(
                    state.receipts.get(`${user}:${row.id}`)?.revision ===
                      row.publication_revision &&
                    state.receipts.get(`${user}:${row.id}`)?.dismissed
                  ),
              )
              .sort((a, b) => b.priority - a.priority)
              .map((announcement) => {
                const receipt = state.receipts.get(
                  `${user}:${announcement.id}`,
                );
                return {
                  announcement,
                  read:
                    receipt?.revision === announcement.publication_revision &&
                    receipt.read,
                  dismissed: false,
                };
              })[0] ?? null,
          announcements: [...state.records.values()]
            .filter(
              (row) =>
                row.status === "published" &&
                (!row.expires_at || Date.parse(row.expires_at) > Date.now()),
            )
            .map((announcement) => {
              const receipt = state.receipts.get(`${user}:${announcement.id}`);
              const current =
                receipt?.revision === announcement.publication_revision;
              return {
                announcement,
                read: current && receipt.read,
                dismissed: current && receipt.dismissed,
              };
            }),
          next_cursor: null,
        },
      });
    }
    expect(request.method()).toBe("POST");
    expect(request.headers()["x-colossus-csrf"]).toBe("1");
    const id = path.split("/").at(-2)!;
    const body = request.postDataJSON() as {
      publication_revision: number;
      read?: true;
      dismissed?: true;
    };
    expect(body.publication_revision).toBe(
      state.records.get(id)!.publication_revision,
    );
    const old = state.receipts.get(`${user}:${id}`);
    const receipt = {
      revision: body.publication_revision,
      read: Boolean(body.read || old?.read),
      dismissed: Boolean(body.dismissed || old?.dismissed),
    };
    state.receipts.set(`${user}:${id}`, receipt);
    return route.fulfill({
      json: {
        publication_revision: receipt.revision,
        read: receipt.read,
        dismissed: receipt.dismissed,
      },
    });
  });
  await page.route("**/api/admin/announcements**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.method() === "GET") {
      return route.fulfill({
        json:
          path === "/api/admin/announcements"
            ? { announcements: [...state.records.values()], next_cursor: null }
            : { announcement: state.records.get(path.split("/").at(-1)!) },
      });
    }
    expect(request.headers()["x-colossus-csrf"]).toBe("1");
    const body = request.postDataJSON() as AnnouncementContent & {
      id?: string;
      revision?: number;
      action: "draft" | "publish" | "archive";
    };
    const id = body.id ?? path.split("/").at(-1)!;
    const old = state.records.get(id);
    if (old) expect(body.revision).toBe(old.revision);
    const { action, revision: _revision, id: _id, ...content } = body;
    const now = new Date().toISOString();
    const next: Announcement = {
      ...content,
      id,
      revision: (old?.revision ?? 0) + 1,
      publication_revision:
        (old?.publication_revision ?? 0) + (action === "publish" ? 1 : 0),
      status:
        action === "publish"
          ? "published"
          : action === "archive"
            ? "archived"
            : "draft",
      created_at: old?.created_at ?? now,
      updated_at: now,
      published_at: action === "publish" ? now : (old?.published_at ?? null),
    };
    state.records.set(id, next);
    state.writes.push(action);
    return route.fulfill({ json: { announcement: next } });
  });
}

test("administrator previews, drafts, publishes, revises and archives an instance update", async ({
  page,
  context,
}, testInfo) => {
  const fixture = await browserFixture(page, { globalAdmin: true });
  const state = instance();
  await announcementFixture(page, state);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/admin/announcements");
  await page
    .getByRole("button", { name: "New announcement", exact: true })
    .click();
  const editor = page.getByRole("region", {
    name: "Announcement editor",
    exact: true,
  });
  await editor.getByLabel(/^Title/).fill("A cleaner workspace");
  await editor
    .getByLabel(/^Banner summary/)
    .fill("See the new shared layout and message tools.");
  await editor.getByLabel(/^Message \(Markdown\)/).fill(message);
  await editor.getByRole("button", { name: "Preview", exact: true }).click();
  const preview = editor.getByLabel("Announcement preview", { exact: true });
  await expect(
    preview.getByRole("heading", { name: "What's new", exact: true }),
  ).toBeVisible();
  await expect(
    preview.getByRole("link", { name: "Read the guide" }),
  ).toHaveAttribute("href", "https://example.com/guide");
  expect(state.writes).toEqual([]);
  await editor.screenshot({
    path: testInfo.outputPath("announcement-admin-preview.png"),
  });
  await editor.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(page.getByText("Draft saved.", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Instance announcement", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "A cleaner workspace", exact: true })
    .click();
  await editor
    .getByRole("button", { name: "Publish to everyone", exact: true })
    .click();
  const banner = page.getByRole("region", {
    name: "Instance announcement",
    exact: true,
  });
  await expect(banner).toContainText("A cleaner workspace");
  await expect(
    page.getByRole("button", {
      name: "Announcements, 1 unread update",
      exact: true,
    }),
  ).toBeVisible();
  await banner
    .getByRole("button", { name: "Read update", exact: true })
    .click();
  const center = page.getByRole("dialog", {
    name: "Announcements",
    exact: true,
  });
  await expect(
    center.getByRole("heading", { name: "A cleaner workspace", exact: true }),
  ).toBeFocused();
  await expect(
    center.getByRole("heading", { name: "What's new", exact: true }),
  ).toBeVisible();
  await center.screenshot({
    path: testInfo.outputPath("announcement-center-desktop.png"),
  });
  await center
    .getByRole("button", {
      name: "Dismiss announcement: A cleaner workspace",
      exact: true,
    })
    .click();
  await expect(
    center.getByText("Banner dismissed", { exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", {
      name: "Announcements, 0 unread updates",
      exact: true,
    }),
  ).toBeFocused();
  await expect(banner).toHaveCount(0);
  await page.reload();
  await expect(
    page.getByRole("button", {
      name: "Announcements, 0 unread updates",
      exact: true,
    }),
  ).toBeVisible();
  await expect(banner).toHaveCount(0);

  // A second page starts with no browser dismissal data and observes server receipts.
  const otherBrowser = await context.browser()!.newContext();
  const otherPage = await otherBrowser.newPage();
  const otherFixture = await browserFixture(otherPage);
  await announcementFixture(otherPage, state);
  await otherPage.goto(
    "http://127.0.0.1:5191/settings?project=browser-project",
  );
  await expect(
    otherPage.getByRole("button", {
      name: "Announcements, 0 unread updates",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    otherPage.getByRole("region", {
      name: "Instance announcement",
      exact: true,
    }),
  ).toHaveCount(0);
  otherFixture.verify();
  await otherBrowser.close();

  await page
    .getByRole("button", { name: "A cleaner workspace", exact: true })
    .click();
  await editor
    .getByLabel(/^Message \(Markdown\)/)
    .fill(`${message}\n\nA second update.`);
  await editor
    .getByRole("button", { name: "Publish changes", exact: true })
    .click();
  await expect(banner).toContainText("A cleaner workspace");
  await expect(
    page.getByRole("button", {
      name: "Announcements, 1 unread update",
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "A cleaner workspace", exact: true })
    .click();
  await editor
    .getByRole("button", { name: "Archive announcement", exact: true })
    .click();
  await expect(banner).toHaveCount(0);
  await expect(
    page.getByText("Announcement archived.", { exact: true }),
  ).toBeVisible();
  expect(state.writes).toEqual(["draft", "publish", "publish", "archive"]);
  fixture.verify();
});

test("ordinary users read Markdown updates at narrow Large-text widths across palettes", async ({
  page,
}, testInfo) => {
  const fixture = await browserFixture(page);
  const state = instance();
  const now = new Date().toISOString();
  state.records.set("update", {
    id: "update",
    revision: 1,
    publication_revision: 1,
    title: "A clearer workspace for everyone",
    summary: "Message tools, code highlighting, and shared panels are here.",
    body: message,
    tone: "success",
    banner: true,
    priority: 50,
    status: "published",
    created_at: now,
    updated_at: now,
    published_at: now,
    expires_at: null,
  });
  await announcementFixture(page, state, "another-operator");
  for (const appearance of appearanceCases) {
    await selectAppearance(page, appearance);
    const banner = page.getByRole("region", {
      name: "Instance announcement",
      exact: true,
    });
    await expect(banner).toContainText("A clearer workspace for everyone");
    await banner.screenshot({
      path: testInfo.outputPath(
        `announcement-banner-mobile-${appearance.id}.png`,
      ),
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await banner
      .getByRole("button", { name: "Read update", exact: true })
      .click();
    const center = page.getByRole("dialog", {
      name: "Announcements",
      exact: true,
    });
    await expect(
      center.getByRole("heading", { name: "What's new", exact: true }),
    ).toBeVisible();
    await expect(
      center.getByRole("button", { name: /Copy code/ }).first(),
    ).toBeVisible();
    expect(
      await center.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
    await center.screenshot({
      path: testInfo.outputPath(
        `announcement-center-mobile-${appearance.id}.png`,
      ),
    });
    await page.keyboard.press("Escape");
  }
  expect(state.writes).toEqual([]);
  fixture.verify();
});

test("the administrator editor and Markdown preview fit narrow Large-text layouts", async ({
  page,
}, testInfo) => {
  const fixture = await browserFixture(page, { globalAdmin: true });
  const state = instance();
  await announcementFixture(page, state);
  for (const appearance of appearanceCases) {
    await selectAppearance(page, appearance);
    await page.goto("/admin/announcements");
    await page
      .getByRole("button", { name: "New announcement", exact: true })
      .click();
    const editor = page.getByRole("region", {
      name: "Announcement editor",
      exact: true,
    });
    await editor
      .getByLabel(/^Title/)
      .fill("A shared update for the whole instance");
    await editor
      .getByLabel(/^Banner summary/)
      .fill("Read about improvements to code, plans, and shared layouts.");
    await editor.getByLabel(/^Message \(Markdown\)/).fill(message);
    await editor.getByRole("button", { name: "Preview", exact: true }).click();
    await expect(
      editor.getByRole("heading", { name: "What's new", exact: true }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await editor.getByRole("heading", { name: "New announcement", exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({path: testInfo.outputPath(`announcement-admin-mobile-${appearance.id}.png`)});
  }
  expect(state.writes).toEqual([]);
  fixture.verify();
});
