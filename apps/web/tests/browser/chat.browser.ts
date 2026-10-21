import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";
import {
  appearanceCases,
  selectAppearance,
  openAppSidebar,
  openConversationList,
} from "./appearance";

for (const appearance of appearanceCases) {
  test(`personal Chat uses configured models and per-user connections in a narrow Large ${appearance.label} layout`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page);
    await selectAppearance(page, appearance);
    await openAppSidebar(page);
    await page.getByRole("link", { name: "Chat", exact: true }).click();
    await expect(page).toHaveURL(/\/chat\?project=browser-project$/);
    await expect(
      page.getByRole("button", { name: "Project chat settings", exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("heading", { name: "What would you like to work on?" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Start chat", exact: true }).click();
    await expect(
      page.getByRole("textbox", { name: "Message Colossus", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Execution agent", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("button", { name: "Manage your connections", exact: true })
      .click();
    const personal = page.getByRole("region", {
      name: "Personal source",
      exact: true,
    });
    const project = page.getByRole("region", {
      name: "Project source",
      exact: true,
    });
    await expect(project).toContainText("Project connection");
    await expect(project.getByRole("button")).toHaveCount(0);
    await personal
      .getByRole("button", { name: "Connect", exact: true })
      .click();
    await expect(
      personal.getByRole("link", { name: "Continue sign-in" }),
    ).toHaveAttribute(
      "href",
      "https://identity.example/authorize?state=synthetic-flow",
    );
    expect(fixture.connectionActions).toEqual(["auth"]);
    fixture.authenticatePersonal();
    await expect(
      personal.getByRole("button", { name: "Disconnect", exact: true }),
    ).toBeVisible();
    await personal
      .getByRole("button", { name: "Disconnect", exact: true })
      .click();
    await expect(
      personal.getByRole("button", { name: "Connect", exact: true }),
    ).toBeVisible();
    expect(fixture.connectionActions).toEqual(["auth", "disconnect"]);
    await page
      .getByRole("button", { name: "Back to chat", exact: true })
      .click();
    const picker = page.getByRole("button", {
      name: /Choose model and reasoning:/,
    });
    await picker.scrollIntoViewIfNeeded();
    await picker.press("Enter");
    const search = page.getByRole("combobox", {
      name: "Search models",
      exact: true,
    });
    await search.fill("Fast browser model");
    await search.press("End");
    await search.press("Enter");
    const slider = page.getByRole("slider", {
      name: "Reasoning effort for next message",
      exact: true,
    });
    await slider.press("Home");
    for (let index = 0; index < 4; index++) await slider.press("ArrowRight");
    await expect(slider).toHaveAttribute("aria-valuetext", "medium");
    await page.keyboard.press("Escape");
    const input = page.getByRole("textbox", {
      name: "Message Colossus",
      exact: true,
    });
    await input.fill("A private chat with a controlled source");
    const initialComposer = await page
      .locator(".chat-new-workspace .shared-conversation-composer")
      .boundingBox();
    expect(initialComposer).not.toBeNull();
    await input.press("Enter");
    await expect.poll(() => fixture.submissions.length).toBe(1);
    expect(fixture.submissions[0]).toMatchObject({
      path: "/api/projects/browser-project/chat/threads",
      request: { model_profile: "fast", reasoning_effort: "medium" },
    });
    expect(fixture.submissions[0]).not.toHaveProperty("node_id");
    await expect(page).toHaveURL(
      /\/projects\/browser-project\/chat\/browser-thread$/,
    );
    await expect(
      page.getByRole("heading", {
        name: "Browser model conversation",
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText("A private chat with a controlled source", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.locator(".thread-detail .shared-conversation-composer"),
    ).toBeVisible();
    const continuedComposer = await page
      .locator(".thread-detail .shared-conversation-composer")
      .boundingBox();
    expect(continuedComposer).not.toBeNull();
    expect(
      Math.abs(initialComposer!.x - continuedComposer!.x),
    ).toBeLessThanOrEqual(2);
    expect(
      Math.abs(initialComposer!.width - continuedComposer!.width),
    ).toBeLessThanOrEqual(2);
    expect(
      Math.abs(
        initialComposer!.y +
          initialComposer!.height -
          continuedComposer!.y -
          continuedComposer!.height,
      ),
    ).toBeLessThanOrEqual(2);
    await expect(
      page.locator(".chat-workspace .shared-work-header:visible"),
    ).toHaveCount(1);
    await expect(
      page.getByRole("button", { name: /Choose model and reasoning:/ }),
    ).toContainText("Fast browser model");
    await expect(page.getByText("Agent inboxes", { exact: true })).toHaveCount(
      0,
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-theme",
      appearance.theme,
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/chat-${appearance.id}-large.png`,
    });
    await openConversationList(page);
    await page.getByRole("link", { name: "New chat", exact: true }).click();
    await expect(page).toHaveURL(/\/chat\?project=browser-project$/);
    await expect(page.getByRole("dialog")).toBeHidden();
    await openConversationList(page);
    await expect(
      page
        .getByRole("navigation", { name: "Your chat history" })
        .getByRole("link", { name: "Browser model conversation", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Only you can open these chats.", { exact: true }),
    ).toBeVisible();
    fixture.verify();
  });
}

for (const appearance of appearanceCases) {
  test(`project chat settings preserve typed configuration in a narrow Large ${appearance.label} layout`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, { administer: true });
    await selectAppearance(page, appearance);
    await openAppSidebar(page);
    await page.getByRole("link", { name: "Chat", exact: true }).click();
    await openConversationList(page);
    await page
      .getByRole("button", { name: "Project settings", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Providers", exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Edit company", exact: true })
      .click();
    await page
      .getByRole("spinbutton", {
        name: /^Request timeout \(ms\)/u,
      })
      .fill("450000");
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect.poll(() => fixture.settingsSubmissions.length).toBe(1);
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Models", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Edit careful", exact: true })
      .click();
    await page
      .getByRole("spinbutton", { name: /^Maximum output \(tokens\)/u })
      .fill("8192");
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect.poll(() => fixture.settingsSubmissions.length).toBe(2);
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Role routing", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Research planner model", exact: true })
      .click();
    await page
      .getByRole("option", { name: "careful · configured-model", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Save project configuration", exact: true })
      .click();
    await expect.poll(() => fixture.settingsSubmissions.length).toBe(3);
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "MCP connections", exact: true })
      .click();
    await page.getByRole("button", { name: "Edit docs", exact: true }).click();
    await page
      .getByRole("textbox", { name: "Allowed tools", exact: true })
      .fill("read_document, search");
    await page
      .getByRole("textbox", { name: "OAuth scopes", exact: true })
      .fill("read, profile");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/chat-settings-${appearance.id}-large.png`,
    });
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect.poll(() => fixture.settingsSubmissions.length).toBe(4);
    expect(fixture.settingsSubmissions[3]).toMatchObject({
      revision: 6,
      providers: {
        profiles: {
          company: { timeoutMs: 450000, generationTimeoutMs: 1200000 },
        },
      },
      models: {
        profiles: { careful: { maxOutputTokens: 8192 } },
        roles: { primary: "careful", research_planner: "careful" },
      },
      mcp: {
        servers: {
          docs: {
            transport: "streamable_http",
            allowedTools: ["read_document", "search"],
            oauth: {
              clientId: "registered-client",
              scopes: ["read", "profile"],
            },
          },
        },
      },
    });
    await expect(
      page.getByText("Project chat configuration saved.", { exact: true }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Back to work", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "Start chat", exact: true }),
    ).toBeVisible();
    fixture.verify();
  });
}
