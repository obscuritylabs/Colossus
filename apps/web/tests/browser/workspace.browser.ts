import { expect, test, type Page } from "@playwright/test";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";
import type { ThreadDetailResponse } from "../../src/api";

async function expectIconRail(page: Page) {
  const rail = page.locator(".control-app-sidebar");
  await expect(rail).toBeVisible();
  await expect(rail).toHaveAttribute("data-collapsible", "icon");
  await expect
    .poll(async () => (await rail.boundingBox())?.width ?? 0)
    .toBeLessThanOrEqual(72);
  const bounds = await rail.boundingBox();
  expect(bounds!.width).toBeGreaterThanOrEqual(44);
  expect(bounds!.width).toBeLessThanOrEqual(72);
  for (const name of ["Home", "Chat", "Fleet", "Projects", "Settings"]) {
    const link = rail.getByRole("link", { name, exact: true });
    await expect(link).toBeVisible();
    await expect(link.locator("svg")).toBeVisible();
    await expect(link.locator("span")).toBeHidden();
    await expect(link).toHaveAttribute("title", name);
  }
  await expect(
    rail.getByRole("button", { name: "Select project", exact: true }),
  ).toBeVisible();
  await expect(
    rail.getByRole("button", { name: "Account menu", exact: true }),
  ).toBeVisible();
}

async function workspaceLayout(page: Page) {
  return page.locator("#control-main").evaluate(() => {
    const measure = (selector: string) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        right: rect.right,
        bottom: rect.bottom,
        clientWidth: element.clientWidth,
        scrollWidth: element.scrollWidth,
        display: style.display,
        minWidth: style.minWidth,
        cssWidth: style.width,
        flex: style.flex,
        grid: style.gridTemplateColumns,
      };
    };
    return {
      frame: measure(".control-plane-frame")!,
      layout: measure(".control-frame-layout")!,
      sidebar: measure(".control-app-sidebar")!,
      main: measure("#control-main")!,
      header: measure(".control-header")!,
      workbench: measure(".control-workbench")!,
      scope: measure(".control-scope-sidebar"),
      content: measure(".control-content")!,
      workspace: measure(".chat-workspace"),
      chat: measure(".chat-main"),
      composer: measure(".shared-conversation-composer"),
      thread: measure(".thread-detail"),
      message: measure(".thread-conversation .shared-conversation-entry"),
      fleet: measure(".fleet-view"),
      host: measure(".host-overview"),
      hostActions: measure(".host-overview-actions"),
      hostWelcome: measure(".host-workspace-welcome h2"),
      nestedMain: Boolean(
        document.querySelector(".control-app-sidebar #control-main"),
      ),
      nestedHistory: Boolean(
        document.querySelector(
          ".control-app-sidebar .chat-history-sidebar, .control-app-sidebar .host-sidebar, .control-app-sidebar .agent-sidebar",
        ),
      ),
      viewport: window.innerWidth,
      styles: [...document.styleSheets].map((sheet) => sheet.href),
    };
  });
}

test("personal hosted chats use the chat workspace from Home, host history, and saved Fleet links", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    hostLabel: "Browser test host",
  });
  await page.setViewportSize({ width: 1210, height: 954 });
  const destination = "/projects/browser-project/chat/browser-thread";
  await page.goto("/?project=browser-project");
  const homeConversation = page
    .locator(".recent-list")
    .getByRole("link", { name: /Browser model conversation/ });
  await expect(homeConversation).toHaveAttribute("href", destination);
  await homeConversation.press("Enter");
  await expect(page).toHaveURL(destination);
  await expect(
    page.getByRole("region", { name: "Personal cloud chat", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Your chat history", exact: true }),
  ).toBeVisible();
  await expect(
    page.locator(".control-app-sidebar .chat-history-sidebar"),
  ).toHaveCount(0);
  await expect(
    page.locator(".control-scope-sidebar .chat-history-sidebar"),
  ).toBeVisible();
  await expect(
    page.getByText("Retained personal chat question", { exact: true }),
  ).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL("/?project=browser-project");

  await page.goto("/projects/browser-project/hosts/browser-host");
  const hostConversation = page
    .locator(".host-workspace-threads")
    .getByRole("link", { name: /Browser model conversation/ });
  await expect(hostConversation).toHaveAttribute("href", destination);
  await hostConversation.click();
  await expect(page).toHaveURL(destination);
  await expect(page.locator(".chat-workspace")).toBeVisible();
  await expect(page.locator(".host-sidebar")).toHaveCount(0);

  // A bookmarked/copied Fleet URL is canonicalized only after its authorized read.
  await page.goto("/fleet?project=browser-project");
  await expect(
    page.getByRole("heading", { name: "Fleet", exact: true }),
  ).toBeVisible();
  await page.goto("/projects/browser-project/threads/browser-thread");
  await expect(page).toHaveURL(destination);
  await expect(page.locator(".chat-workspace")).toBeVisible();
  await expect(
    page.getByRole("heading", {
      name: "Browser model conversation",
      exact: true,
    }),
  ).toHaveCount(1);
  await expect(
    page.getByText("Retained personal chat question", { exact: true }),
  ).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL("/fleet?project=browser-project");
  fixture.verify();
});

test("agent conversation hrefs preserve personal hosted chat presentation", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { managedChat: true });
  await page.setViewportSize({ width: 1210, height: 954 });
  await page.goto(`/projects/browser-project/agents/${fixture.nodeId}/threads`);
  const destination = "/projects/browser-project/chat/browser-thread";
  const sidebarConversation = page
    .locator(".agent-sidebar")
    .getByRole("link", { name: /Browser model conversation/ });
  const listedConversation = page
    .locator(".workspace-resource-body")
    .getByRole("link", { name: /Browser model conversation/ });
  await expect(sidebarConversation).toHaveAttribute("href", destination);
  await expect(listedConversation).toHaveAttribute("href", destination);
  await listedConversation.click();
  await expect(page).toHaveURL(destination);
  await expect(page.locator(".chat-workspace")).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Your chat history", exact: true }),
  ).toBeVisible();
  fixture.verify();
});

test("narrow app navigation and conversation history open independently with keyboard focus", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { managedChat: true });
  await page.goto("/projects/browser-project/chat/browser-thread");
  const input = page.getByRole("textbox", {
    name: "Message the agent",
    exact: true,
  });
  await input.fill("Keep this draft while opening navigation");
  const appToggle = page.getByRole("button", {
    name: "Toggle app sidebar",
    exact: true,
    includeHidden: true,
  });
  const listToggle = page.getByRole("button", {
    name: "Toggle conversation list",
    exact: true,
    includeHidden: true,
  });
  await listToggle.focus();
  await listToggle.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("navigation", { name: "Your chat history", exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("link", { name: "Home", exact: true }),
  ).toHaveCount(0);
  await expect(listToggle).toHaveAttribute("aria-expanded", "true");
  await expect.poll(() => dialog.locator(":focus").count()).toBe(1);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(listToggle).toBeFocused();
  await expect(input).toHaveValue("Keep this draft while opening navigation");

  await appToggle.focus();
  await appToggle.press("Enter");
  await expect(
    dialog.getByRole("link", { name: "Home", exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("navigation", { name: "Your chat history", exact: true }),
  ).toHaveCount(0);
  const accountMenu = dialog.getByRole("button", {
    name: "Account menu",
    exact: true,
  });
  await accountMenu.focus();
  await accountMenu.press("Enter");
  await expect(
    page.getByRole("menuitem", { name: "Sign out", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page
        .getByRole("menu")
        .evaluate((menu) => menu.contains(document.activeElement)),
    )
    .toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toBeHidden();
  await expect(accountMenu).toBeFocused();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(appToggle).toBeFocused();
  await expect(input).toHaveValue("Keep this draft while opening navigation");

  await listToggle.press("Enter");
  await dialog
    .getByRole("link", { name: "New chat", exact: true })
    .press("Enter");
  await expect(page).toHaveURL("/chat?project=browser-project");
  await expect(dialog).toBeHidden();
  await expect(
    page.getByRole("textbox", { name: "Message Colossus", exact: true }),
  ).toBeVisible();
  await appToggle.press("Enter");
  await dialog
    .getByRole("button", { name: "Account menu", exact: true })
    .press("Enter");
  await page
    .getByRole("menuitem", { name: "Settings", exact: true })
    .press("Enter");
  await expect(dialog).toBeHidden();
  await page.locator("#appearance-text-size").click();
  await page.getByRole("option", { name: "Large", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-text-size", "large");
  await appToggle.click();
  await dialog
    .getByRole("button", { name: "Select project", exact: true })
    .press("Enter");
  await page
    .getByRole("menuitem", { name: "Manage projects", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("heading", { name: "Projects", exact: true }),
  ).toBeVisible();
  await expect(dialog).toBeHidden();
  await appToggle.click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(appToggle).toBeFocused();
  fixture.verify();
});

test("compact Large chat history keeps keyboard filters, title search, and drafts usable", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { managedChat: true });
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.web.appearance.v1",
      JSON.stringify({ colorTheme: "dark", textSize: "large" }),
    ),
  );
  const historyPaths: URL[] = [];
  await page.route(
    "**/api/projects/browser-project/chat/threads?**",
    async (route) => {
      const url = new URL(route.request().url());
      historyPaths.push(url);
      const archived = url.searchParams.get("archived") === "true";
      const title = archived
        ? "Archived browser conversation"
        : "Browser model conversation";
      const matches =
        !url.searchParams.get("query") ||
        title
          .toLowerCase()
          .includes(url.searchParams.get("query")!.toLowerCase());
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          threads: matches
            ? [
                {
                  thread_id: archived ? "archived-thread" : "browser-thread",
                  project_id: "browser-project",
                  owner_user_id: "browser-user",
                  node_id: fixture.nodeId,
                  title,
                  created_at: "2026-10-16T10:00:00Z",
                  updated_at: "2026-10-16T10:00:00Z",
                  revision: 1,
                  archived,
                  session_id: "browser-session",
                  sync_status: "current",
                  source: "cloud",
                  can_continue: true,
                },
              ]
            : [],
          next_cursor: null,
        }),
      });
    },
  );
  await page.goto("/projects/browser-project/chat/browser-thread");
  const draft = page.getByRole("textbox", {
    name: "Message the agent",
    exact: true,
  });
  await draft.fill("Keep this draft while reviewing chat history");
  const toggle = page.getByRole("button", {
    name: "Toggle conversation list",
    exact: true,
  });
  await toggle.press("Enter");
  const dialog = page.getByRole("dialog");
  const history = dialog.getByRole("region", {
    name: "Your chats",
    exact: true,
  });
  await expect(
    history.getByRole("heading", { name: "Chats", exact: true }),
  ).toBeVisible();
  const selected = history.getByRole("link", {
    name: "Browser model conversation",
    exact: true,
  });
  await expect(selected).toHaveAttribute("aria-current", "page");
  await expect(selected).toHaveAccessibleDescription(/Updated /);
  const options = history.getByRole("button", {
    name: "Chat options",
    exact: true,
  });
  await options.press("Enter");
  const refresh = page.getByRole("menuitem", {
    name: "Refresh chats",
    exact: true,
  });
  await expect(refresh).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(options).toBeFocused();
  await expect(dialog).toBeVisible();
  await options.press("Enter");
  const requestsBeforeRefresh = historyPaths.length;
  await refresh.press("Enter");
  await expect
    .poll(() => historyPaths.length)
    .toBeGreaterThan(requestsBeforeRefresh);
  await expect(dialog).toBeVisible();
  const view = history.getByRole("radiogroup", {
    name: "Chat history view",
    exact: true,
  });
  const open = view.getByRole("radio", { name: "Open", exact: true });
  const archived = view.getByRole("radio", { name: "Archived", exact: true });
  await open.focus();
  await open.press("ArrowRight");
  await expect(archived).toBeFocused();
  await expect(archived).toBeChecked();
  await expect(
    history.getByRole("link", {
      name: "Archived browser conversation",
      exact: true,
    }),
  ).toBeVisible();
  await expect
    .poll(() => historyPaths.at(-1)?.searchParams.get("archived"))
    .toBe("true");
  const search = history.getByRole("searchbox", {
    name: "Search your chats",
    exact: true,
  });
  await search.fill("missing conversation");
  await expect(
    history.getByText("No matching chats", { exact: true }),
  ).toBeVisible();
  await expect
    .poll(() => historyPaths.at(-1)?.searchParams.get("query"))
    .toBe("missing conversation");
  expect(historyPaths.at(-1)?.searchParams.get("archived")).toBe("true");
  await search.fill("");
  await archived.focus();
  await archived.press("ArrowLeft");
  await expect(open).toBeChecked();
  await expect(selected).toBeVisible();
  await expect
    .poll(() => historyPaths.at(-1)?.searchParams.has("query"))
    .toBe(false);
  await page.screenshot({
    path: "output/playwright/chat-history-narrow-large.png",
  });
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(toggle).toBeFocused();
  await expect(draft).toHaveValue(
    "Keep this draft while reviewing chat history",
  );
  expect(historyPaths.every((url) => !url.searchParams.has("user_id"))).toBe(
    true,
  );
  fixture.verify();
});

test("collapsed app icons keep navigation available and expand with the keyboard", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    administer: true,
  });
  await page.setViewportSize({ width: 1210, height: 954 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.web.appearance.v1",
      JSON.stringify({ colorTheme: "dark", textSize: "large" }),
    ),
  );
  await page.goto("/projects/browser-project/chat/browser-thread");
  const input = page.getByRole("textbox", {
    name: "Message the agent",
    exact: true,
  });
  await input.fill("Keep this draft while collapsing the app menu");
  const appToggle = page.getByRole("button", {
    name: "Toggle app sidebar",
    exact: true,
  });
  await appToggle.click();
  await expect(appToggle).toHaveAttribute("aria-expanded", "false");
  await expectIconRail(page);
  await expect(page.locator(".control-scope-sidebar")).toBeVisible();
  await expect(input).toHaveValue(
    "Keep this draft while collapsing the app menu",
  );
  const rail = page.locator(".control-app-sidebar");
  await expect(rail).toHaveCSS("transition-duration", "0s");
  const projectMenu = rail.getByRole("button", {
    name: "Select project",
    exact: true,
  });
  await projectMenu.press("Enter");
  await expect(
    page.getByRole("menuitem", { name: "Browser test project", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toBeHidden();
  await expect(projectMenu).toBeFocused();
  const accountMenu = rail.getByRole("button", {
    name: "Account menu",
    exact: true,
  });
  await accountMenu.press("Enter");
  for (const name of ["Settings", "Sign out"]) {
    await expect(
      page.getByRole("menuitem", { name, exact: true }),
    ).toBeVisible();
  }
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toBeHidden();
  await expect(accountMenu).toBeFocused();
  await expect(input).toHaveValue(
    "Keep this draft while collapsing the app menu",
  );
  await rail.getByRole("link", { name: "Fleet", exact: true }).press("Enter");
  await expect(
    page.getByRole("heading", { name: "Fleet", exact: true }),
  ).toBeVisible();
  await expectIconRail(page);
  await rail
    .getByRole("link", { name: "Settings", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("navigation", { name: "Settings sections", exact: true }),
  ).toBeVisible();
  await expectIconRail(page);
  await page.keyboard.press("ControlOrMeta+b");
  await expect(appToggle).toHaveAttribute("aria-expanded", "true");
  await expect(
    rail.getByRole("button", { name: "Select project", exact: true }),
  ).toBeVisible();
  await expect(
    rail.getByRole("link", { name: "Chat", exact: true }).locator("span"),
  ).toBeVisible();
  fixture.verify();
});

test("short desktop Large navigation scrolls to its destinations and footer", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { managedChat: true });
  await page.setViewportSize({ width: 1024, height: 400 });
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.web.appearance.v1",
      JSON.stringify({ colorTheme: "dark", textSize: "large" }),
    ),
  );
  await page.goto("/projects/browser-project/chat/browser-thread");
  const sidebar = page.locator(".control-app-sidebar");
  await expect(sidebar).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-text-size", "large");
  await expect(
    sidebar.getByRole("link", { name: "Home", exact: true }),
  ).toBeInViewport();
  await sidebar.hover();
  await page.mouse.wheel(0, 800);
  const accountMenu = sidebar.getByRole("button", {
    name: "Account menu",
    exact: true,
  });
  await expect(accountMenu).toBeInViewport();
  for (const name of ["Home", "Chat", "Fleet", "Projects", "Settings"]) {
    const destination = sidebar.getByRole("link", { name, exact: true });
    await destination.focus();
    await expect(destination).toBeFocused();
    await expect(destination).toBeInViewport();
  }
  await accountMenu.focus();
  await expect(accountMenu).toBeFocused();
  await expect(accountMenu).toBeInViewport();
  await accountMenu.press("Enter");
  const signOut = page.getByRole("menuitem", { name: "Sign out", exact: true });
  await expect(signOut).toBeInViewport();
  await signOut.focus();
  await expect(signOut).toBeFocused();
  await expect(signOut).toBeInViewport();
  await page.keyboard.press("Escape");
  await expect(accountMenu).toBeFocused();
  fixture.verify();
});

test("built workspace keeps chat and Fleet inside the main pane with expanded and collapsed navigation", async ({
  page,
  browserName,
}) => {
  const fixture = await browserFixture(page, {
    administer: true,
    projectName: "Web Chat shared SQL development",
    hostLabel: "Olabs-Alexander-Rymdeko-Harvey-51",
  });
  await page.setViewportSize({ width: 1210, height: 954 });
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.web.appearance.v1",
      JSON.stringify({ colorTheme: "dark", textSize: "large" }),
    ),
  );
  const states: Array<{
    name: string;
    layout: Awaited<ReturnType<typeof workspaceLayout>>;
  }> = [];
  async function capture(name: string) {
    const layout = await workspaceLayout(page);
    states.push({ name, layout });
    await test.info().attach(`workspace-${name}`, {
      body: JSON.stringify(layout, null, 2),
      contentType: "application/json",
    });
    await page.screenshot({
      path: `output/playwright/workspace-bounds-${browserName}-${name}.png`,
    });
  }
  await page.goto("/projects/browser-project/threads/browser-thread");
  await expect(
    page.getByRole("heading", {
      name: "Browser model conversation",
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-text-size", "large");
  await expect(
    page.getByRole("complementary", {
      name: "Conversation navigation",
      exact: true,
    }),
  ).toBeVisible();
  await capture("expanded-plain-thread-cold");
  await page
    .getByRole("button", { name: "Toggle app sidebar", exact: true })
    .click();
  await expectIconRail(page);
  await expect(page.locator(".control-scope-sidebar")).toBeVisible();
  await capture("collapsed-plain-thread-cold");
  await page.goto("/projects/browser-project/hosts/browser-host");
  await expect(
    page.getByRole("heading", {
      name: "Olabs-Alexander-Rymdeko-Harvey-51",
      exact: true,
    }),
  ).toBeVisible();
  await capture("expanded-host-cold");
  await page
    .getByRole("button", { name: "Toggle app sidebar", exact: true })
    .click();
  await expectIconRail(page);
  await capture("collapsed-host-cold");
  await page
    .getByRole("button", { name: "Toggle app sidebar", exact: true })
    .click();
  await page.getByRole("link", { name: "Chat", exact: true }).click();
  await page.getByRole("button", { name: "Start chat", exact: true }).click();
  const composer = page.getByRole("textbox", {
    name: "Message Colossus",
    exact: true,
  });
  await expect(composer).toBeVisible();
  await page
    .getByRole("button", { name: "Toggle conversation list", exact: true })
    .click();
  await expect(page.locator(".control-scope-sidebar")).toBeHidden();
  await expect(page.locator(".control-app-sidebar")).toBeVisible();
  await capture("expanded-app-collapsed-list");
  await page
    .getByRole("button", { name: "Toggle conversation list", exact: true })
    .click();
  await expect(page.locator(".control-scope-sidebar")).toBeVisible();
  await capture("expanded-new");
  await page
    .getByRole("button", { name: "Toggle app sidebar", exact: true })
    .click();
  await expectIconRail(page);
  await capture("collapsed-new");
  await composer.fill("Check the production conversation layout");
  await composer.press("Enter");
  await expect(
    page.getByRole("textbox", { name: "Message the agent", exact: true }),
  ).toBeVisible();
  await capture("collapsed-conversation");
  await page
    .getByRole("button", { name: "Toggle app sidebar", exact: true })
    .click();
  await expect(page.locator(".control-app-sidebar")).toBeVisible();
  await capture("expanded-conversation");
  await page.getByRole("link", { name: "Fleet", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Fleet", exact: true }),
  ).toBeVisible();
  await capture("expanded-fleet");
  await page
    .getByRole("button", { name: "Toggle app sidebar", exact: true })
    .click();
  await expectIconRail(page);
  await capture("collapsed-fleet");
  for (const { name, layout } of states) {
    expect(layout.nestedMain, `${name}: main is outside app navigation`).toBe(
      false,
    );
    expect(
      layout.nestedHistory,
      `${name}: history is outside app navigation`,
    ).toBe(false);
    expect(
      layout.main.right,
      `${name}: main right boundary`,
    ).toBeLessThanOrEqual(layout.viewport);
    expect(
      layout.workbench.width,
      `${name}: workbench fills main`,
    ).toBeGreaterThan(layout.main.width - 4);
    expect(
      layout.content.right,
      `${name}: content ends at main boundary`,
    ).toBeLessThanOrEqual(layout.main.right);
    if (layout.scope?.width) {
      expect(
        layout.scope.x,
        `${name}: list starts inside main`,
      ).toBeGreaterThanOrEqual(layout.main.x);
      expect(
        layout.scope.right,
        `${name}: list is beside content`,
      ).toBeLessThanOrEqual(layout.content.x);
      expect(
        layout.content.width + layout.scope.width,
        `${name}: list and content fill workbench`,
      ).toBeGreaterThan(layout.workbench.width - 4);
    } else {
      expect(
        layout.content.width,
        `${name}: content fills workbench without list`,
      ).toBeGreaterThan(layout.workbench.width - 4);
    }
    expect(
      layout.content.scrollWidth,
      `${name}: no horizontal page overflow`,
    ).toBeLessThanOrEqual(layout.content.clientWidth + 1);
    expect(
      layout.header.scrollWidth,
      `${name}: window header fits main`,
    ).toBeLessThanOrEqual(layout.header.clientWidth + 1);
    const conversation = layout.chat ?? layout.thread;
    if (conversation) {
      expect(
        conversation.width,
        `${name}: conversation fills main`,
      ).toBeGreaterThan(layout.content.width - 4);
      expect(
        layout.composer!.width,
        `${name}: bounded composer fills available column`,
      ).toBeGreaterThanOrEqual(Math.min(700, layout.content.width - 64));
      expect(
        layout.composer!.x,
        `${name}: composer inside main`,
      ).toBeGreaterThan(layout.content.x);
      expect(
        layout.composer!.right,
        `${name}: composer within main`,
      ).toBeLessThan(layout.content.right);
    }
    if (layout.message) {
      expect(
        layout.message.x,
        `${name}: message starts within main`,
      ).toBeGreaterThan(layout.content.x);
      expect(layout.message.right, `${name}: message fits main`).toBeLessThan(
        layout.content.right,
      );
      expect(
        layout.message.width,
        `${name}: message uses conversation column`,
      ).toBeGreaterThanOrEqual(Math.min(700, layout.content.width - 64));
    }
    if (layout.host) {
      expect(
        layout.host.right,
        `${name}: host content within main`,
      ).toBeLessThanOrEqual(layout.main.right);
      expect(
        layout.hostActions!.right,
        `${name}: host actions within main`,
      ).toBeLessThan(layout.main.right);
      expect(
        Math.abs(
          layout.hostWelcome!.x +
            layout.hostWelcome!.width / 2 -
            layout.content.x -
            layout.content.width / 2,
        ),
        `${name}: host welcome centered in main`,
      ).toBeLessThanOrEqual(2);
    }
  }
  await expect(
    page.locator(".control-app-sidebar .chat-workspace"),
  ).toHaveCount(0);
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`released session views preserve data truth and host links in a narrow Large ${appearance.label} layout`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      plan: true,
    });
    const output =
      "## Inspect released session records\n\n1. Review the retained records before making changes.\n\n## Sources\n\n- [R1] Official documentation — https://example.test/documentation/session-resources\n- [R2] Workspace evidence — repo://docs/develop/session-resources.md";
    const detail: ThreadDetailResponse = {
      thread: {
        thread_id: "browser-thread",
        project_id: "browser-project",
        node_id: fixture.nodeId,
        owner_user_id: "browser-user",
        title: "Browser model conversation",
        created_at: "2026-10-16T10:00:00Z",
        updated_at: "2026-10-16T10:01:00Z",
        revision: 1,
        archived: false,
        source: "cloud",
        sync_status: "current",
        can_continue: true,
        session_id: "browser-session",
      },
      tasks: [
        {
          task_id: "browser-plan-task",
          project_id: "browser-project",
          node_id: fixture.nodeId,
          subject: "browser-user",
          run_id: "browser-plan-run",
          last_sequence: 1,
          revision: 1,
          request: {
            input: [{ text: "Inspect session records" }],
            mode: "plan",
            role: "primary",
          },
          snapshot: {
            run: {
              run_id: "browser-plan-run",
              session_id: "browser-session",
              title: "Inspect session records",
              status: "completed",
              mode: "plan",
              role: "primary",
              last_sequence: 1,
              created_at: "2026-10-16T10:00:00Z",
              updated_at: "2026-10-16T10:01:00Z",
              terminal: {
                result: { output, plan_id: "browser-plan", plan_revision: 2 },
              },
            },
            pending_interactions: [],
          },
        },
      ],
      messages: [],
    };
    await page.route(
      "**/api/projects/browser-project/threads/browser-thread",
      (route) =>
        route.fulfill({
          contentType: "application/json",
          body: JSON.stringify(detail),
        }),
    );
    await selectAppearance(page, appearance);
    await page.goto("/projects/browser-project/chat/browser-thread");
    const tabs = page.getByRole("navigation", {
      name: "Session views",
      exact: true,
    });
    await tabs.getByRole("button", { name: "Topology", exact: true }).click();
    await expect(
      page.getByText("Session map unavailable", { exact: true }),
    ).toBeVisible();
    for (const name of ["Fit", "Collapse all", "Layers"]) {
      await expect(page.getByRole("button", { name, exact: true })).toHaveCount(
        0,
      );
    }
    await page.screenshot({
      path: `output/playwright/session-topology-${appearance.id}-large.png`,
    });
    await tabs.getByRole("button", { name: "Plans", exact: true }).click();
    const plans = page.locator(".session-plan-list");
    await expect(
      plans.getByText("Status not reported", { exact: true }),
    ).toBeVisible();
    await expect(
      plans.getByRole("button", { name: "Revise", exact: true }),
    ).toBeDisabled();
    await expect(
      plans.getByRole("heading", {
        name: "Inspect released session records",
        exact: true,
      }),
    ).toBeVisible();
    await page.screenshot({
      path: `output/playwright/session-plans-${appearance.id}-large.png`,
    });
    await plans.getByRole("button", { name: "Read plan", exact: true }).click();
    await expect(
      page.getByRole("heading", {
        name: "Inspect session records",
        exact: true,
      }),
    ).toBeVisible();
    await page
      .getByRole("dialog", { name: "Plan", exact: true })
      .getByRole("button", { name: "Close plan", exact: true })
      .click();
    await tabs.getByRole("button", { name: "Snapshots", exact: true }).click();
    await expect(
      page.getByLabel("Snapshot count unavailable", { exact: true }),
    ).toHaveText("—");
    await expect(
      page.getByText("Context snapshots are unavailable", { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: `output/playwright/session-snapshots-${appearance.id}-large.png`,
    });
    await tabs.getByRole("button", { name: "Sources", exact: true }).click();
    const sources = page.locator(".session-source-list");
    await expect(
      sources.getByText("Official documentation", { exact: true }),
    ).toBeVisible();
    await expect(
      sources.getByRole("link", {
        name: "https://example.test/documentation/session-resources",
        exact: true,
      }),
    ).toHaveAttribute("rel", "noopener noreferrer");
    await expect(
      sources.getByRole("button", { name: "Open file", exact: true }),
    ).toHaveCount(0);
    await page.screenshot({
      path: `output/playwright/session-sources-${appearance.id}-large.png`,
    });
    await tabs.getByRole("button", { name: "Resources", exact: true }).click();
    await expect(
      page.getByText("Durable records are unavailable", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", {
        name: /Context snapshots Immutable compacted context/u,
      }),
    ).toContainText("—");
    await page.screenshot({
      path: `output/playwright/session-resources-${appearance.id}-large.png`,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    fixture.verify();
  });
}

for (const appearance of appearanceCases.slice(0, 2)) {
  test(`Desktop ${appearance.label} workspace preserves drafts and uses one app window for chat and settings`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, { administer: true });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await selectAppearance(page, appearance);
    await page.getByRole("link", { name: "Chat", exact: true }).press("Enter");
    await expect(page.locator(".control-app-sidebar")).toBeVisible();
    await expect(
      page
        .locator('.control-app-sidebar [data-slot="sidebar-header"]')
        .getByRole("button", { name: "Select project", exact: true }),
    ).toBeVisible();
    await expect(page.locator(".control-primary-rail")).toHaveCount(0);
    await expect(page.locator(".chat-history")).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: "New chat", exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Start chat", exact: true }).click();
    const newInput = page.getByRole("textbox", {
      name: "Message Colossus",
      exact: true,
    });
    await newInput.fill("Keep my unsent question");
    await page
      .getByRole("button", { name: "Manage your connections", exact: true })
      .press("Enter");
    await expect(
      page.getByRole("heading", { name: "Your connections", exact: true }),
    ).toBeVisible();
    await expect(newInput).toBeHidden();
    await page
      .getByRole("button", { name: "Back to chat", exact: true })
      .press("Enter");
    await expect(newInput).toHaveValue("Keep my unsent question");
    const before = await page
      .locator(".chat-new-workspace .shared-conversation-composer")
      .boundingBox();
    await page.screenshot({
      path: `output/playwright/workspace-new-${appearance.id}.png`,
    });
    await newInput.press("Enter");
    await expect.poll(() => fixture.submissions.length).toBe(1);
    const followup = page.getByRole("textbox", {
      name: "Message the agent",
      exact: true,
    });
    await expect(followup).toBeVisible();
    const after = await page
      .locator(".thread-detail .shared-conversation-composer")
      .boundingBox();
    expect(before).not.toBeNull();
    expect(after).not.toBeNull();
    expect(Math.abs(before!.width - after!.width)).toBeLessThanOrEqual(2);
    expect(Math.abs(before!.x - after!.x)).toBeLessThanOrEqual(2);
    expect(
      Math.abs(before!.y + before!.height - after!.y - after!.height),
    ).toBeLessThanOrEqual(2);
    await expect(
      page.locator(".chat-workspace .shared-work-header:visible"),
    ).toHaveCount(1);
    await expect(
      page.getByRole("button", { name: "Inboxes", exact: true }),
    ).toHaveCount(0);
    await followup.fill("Preserve this follow-up while I review settings");
    await page
      .getByRole("button", { name: "Chat options", exact: true })
      .press("Enter");
    await page
      .getByRole("menuitem", { name: "Project settings", exact: true })
      .press("Enter");
    const sections = page.getByRole("navigation", {
      name: "Settings sections",
    });
    await expect(
      page.getByRole("heading", { name: "Providers", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Edit careful", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("button", { name: "Edit company", exact: true })
      .click();
    const timeout = page.getByRole("spinbutton", {
      name: /^Request timeout \(ms\)/u,
    });
    await timeout.fill("450000");
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    expect(fixture.settingsSubmissions).toHaveLength(0);
    await expect(
      page.getByRole("button", {
        name: "Save project configuration",
        exact: true,
      }),
    ).toBeDisabled();
    await page
      .getByRole("button", { name: "Edit company", exact: true })
      .click();
    await expect(timeout).toHaveValue("300000");
    await timeout.fill("450000");
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect.poll(() => fixture.settingsSubmissions.length).toBe(1);
    await sections
      .getByRole("button", { name: "Models", exact: true })
      .press("Enter");
    await expect(
      page.getByRole("heading", { name: "Models", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Unsaved configuration changes", { exact: true }),
    ).toHaveCount(0);
    await sections
      .getByRole("button", { name: "Providers", exact: true })
      .press("Enter");
    await page
      .getByRole("button", { name: "Edit company", exact: true })
      .click();
    await expect(timeout).toHaveValue("450000");
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.screenshot({
      path: `output/playwright/workspace-settings-${appearance.id}.png`,
    });
    expect(fixture.settingsSubmissions).toHaveLength(1);
    expect(fixture.settingsSubmissions[0]).toMatchObject({
      providers: { profiles: { company: { timeoutMs: 450000 } } },
    });
    await page
      .getByRole("button", { name: "Back to work", exact: true })
      .press("Enter");
    await expect(followup).toHaveValue(
      "Preserve this follow-up while I review settings",
    );
    await expect(
      page.getByRole("heading", {
        name: "Browser model conversation",
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.locator("#control-main")).toHaveCount(1);
    await page.screenshot({
      path: `output/playwright/workspace-conversation-${appearance.id}.png`,
    });
    fixture.verify();
  });
}
