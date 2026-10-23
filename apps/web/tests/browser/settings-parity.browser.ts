import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";
import {
  appearanceCases,
  selectAppearance,
  openAppSidebar,
  openConversationList,
} from "./appearance";

for (const appearance of appearanceCases) {
  test(`shared stop stays in the composer and queues drafts in ${appearance.label}`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      running: true,
    });
    await selectAppearance(page, appearance);
    await page.goto("/projects/browser-project/chat/browser-thread");
    const composer = page.locator(".shared-conversation-composer");
    const stop = composer.getByRole("button", {
      name: "Stop response",
      exact: true,
    });
    await expect(stop).toBeVisible();
    await expect(page.locator(".thread-run-controls")).toHaveCount(0);
    await expect(
      composer.getByRole("button", { name: "Queue message", exact: true }),
    ).toHaveCount(0);
    const draft = page.getByRole("textbox", {
      name: "Message the agent",
      exact: true,
    });
    await draft.fill("Follow up after this response");
    await expect(
      composer.getByRole("button", { name: "Queue message", exact: true }),
    ).toBeEnabled();
    await expect(stop).toBeEnabled();
    await page.screenshot({
      path: `output/playwright/composer-stop-${appearance.id}.png`,
    });
    await stop.press("Enter");
    await expect.poll(() => fixture.cancellationIds.length).toBe(1);
    await expect(stop).toBeHidden();
    await expect(draft).toHaveValue("Follow up after this response");
    await expect(
      composer.getByRole("button", { name: "Send message", exact: true }),
    ).toBeEnabled();
    fixture.verify();
  });

  test(`shared provider catalog and encrypted credential forms work in ${appearance.label}`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, { administer: true });
    await selectAppearance(page, appearance);
    await openAppSidebar(page);
    await page.getByRole("link", { name: "Chat", exact: true }).click();
    await openConversationList(page);
    await page
      .getByRole("button", { name: "Chat options", exact: true })
      .click();
    await page
      .getByRole("menuitem", { name: "Project settings", exact: true })
      .click();
    const sections = page.getByRole("navigation", {
      name: "Settings sections",
    });
    await page
      .getByRole("button", { name: "Add provider", exact: true })
      .click();
    await page.getByRole("combobox", { name: "Provider", exact: true }).click();
    await page.getByRole("option", { name: "Anthropic", exact: true }).click();
    await expect(
      page.getByRole("textbox", { name: /^Profile ID/u }),
    ).toHaveValue("anthropic");
    await expect(
      page.getByRole("textbox", { name: /^Endpoint URL/u }),
    ).toHaveValue("https://api.anthropic.com/v1");
    await expect(
      page.getByRole("combobox", { name: "Authentication", exact: true }),
    ).toContainText("Personal account");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    const settingsBounds = await page
      .locator(".web-settings-window")
      .evaluate((element) => {
        const rect = element.getBoundingClientRect();
        return {
          left: rect.left,
          right: rect.right,
          width: innerWidth,
          ancestors: [
            ...document.querySelectorAll(
              ".web-settings-window, .managed-settings-shell, .settings-frame-layout, .settings-main, .chat-settings-editor",
            ),
          ].map((node) => ({
            className: node.className,
            rect: node.getBoundingClientRect().toJSON(),
            scrollWidth: node.scrollWidth,
            clientWidth: node.clientWidth,
            scrollLeft: node.scrollLeft,
          })),
        };
      });
    expect(
      settingsBounds.left,
      JSON.stringify(settingsBounds),
    ).toBeGreaterThanOrEqual(0);
    expect(
      settingsBounds.right,
      JSON.stringify(settingsBounds),
    ).toBeLessThanOrEqual(settingsBounds.width);
    for (const bounds of settingsBounds.ancestors) {
      expect(
        bounds.rect.x,
        JSON.stringify(settingsBounds),
      ).toBeGreaterThanOrEqual(0);
      expect(
        bounds.rect.right,
        JSON.stringify(settingsBounds),
      ).toBeLessThanOrEqual(settingsBounds.width);
    }
    await page.screenshot({
      path: `output/playwright/provider-setup-${appearance.id}.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await page
      .getByRole("button", { name: "Edit company", exact: true })
      .click();
    const manageCredentials = page.getByRole("button", {
      name: /^(Choose or add credential|Save and choose credential)$/u,
      exact: true,
    });
    const providerUrl = page.getByRole("textbox", {
      name: /^Endpoint URL/u,
    });
    await providerUrl.fill("https://changed.example/v1");
    await expect(manageCredentials).toBeEnabled();
    await providerUrl.fill("https://models.example/v1");
    await manageCredentials.click();
    await expect(
      page.locator("input[aria-label='API key or token']"),
    ).toBeInViewport();
    await expect(page.locator("form.provider-editor")).toHaveCount(0);
    await page
      .getByRole("textbox", { name: "Display label", exact: true })
      .fill("Project API key");
    await expect(
      page.locator("input[aria-label='API key or token']"),
    ).toBeEnabled();
    await sections.getByRole("button", { name: "Models", exact: true }).click();
    await page
      .getByRole("button", { name: "Edit careful", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Load models", exact: true })
      .click();
    await page
      .getByRole("region", { name: "Available models", exact: true })
      .getByRole("searchbox", { name: "Search models", exact: true })
      .fill("Catalog Reasoner");
    await page.getByRole("button", { name: /^Catalog Reasoner/ }).click();
    await expect(
      page.getByRole("textbox", { name: /^Model identifier/u }),
    ).toHaveValue("catalog-reasoner");
    await expect(
      page.getByRole("spinbutton", {
        name: /^Context window \(tokens\)/u,
      }),
    ).toHaveValue("200000");
    await expect(
      page.getByRole("spinbutton", {
        name: /^Maximum output \(tokens\)/u,
      }),
    ).toHaveValue("30000");
    await page.screenshot({
      path: `output/playwright/provider-models-${appearance.id}.png`,
      fullPage: true,
    });
    if (appearance.id === "dark") {
      await page.setViewportSize({ width: 1280, height: 900 });
      await page.locator(".settings-main").evaluate((element) => {
        element.scrollTop = 0;
      });
      await page.screenshot({
        path: "output/playwright/provider-models-desktop.png",
      });
      await page.setViewportSize({ width: 390, height: 844 });
    }
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect.poll(() => fixture.settingsSubmissions.length).toBe(1);
    expect(fixture.settingsSubmissions[0]).toMatchObject({
      models: {
        profiles: {
          careful: {
            model: "catalog-reasoner",
            capabilities: { declared: { toolCalls: true } },
          },
        },
      },
    });
    await sections
      .getByRole("button", { name: "Secrets", exact: true })
      .click();
    // Password inputs have no implicit textbox role.
    const projectInput = page.locator("input[aria-label='API key or token']");
    // The selected connection reopens its editor after the credential catalog loads.
    await expect(page.locator(".shared-credential-editor")).toBeVisible();
    await page.screenshot({
      path: `output/playwright/credentials-project-editor-${appearance.id}.png`,
      fullPage: true,
    });
    await page
      .getByRole("textbox", { name: "Display label", exact: true })
      .fill("Project API key");
    await projectInput.fill("synthetic-shared-browser-secret");
    await page
      .getByRole("button", { name: "Save credential", exact: true })
      .click();
    await expect.poll(() => fixture.credentialWrites.length).toBe(1);
    await expect(projectInput).toHaveCount(0);
    await expect(
      page.getByText("Project credential saved.", { exact: true }),
    ).toBeVisible();
    expect(fixture.credentialWrites[0]!.body).toEqual({
      revision: 0,
      label: "Project API key",
      kind: "api_key",
      secret: "synthetic-shared-browser-secret",
      attachment: {
        kind: "provider",
        connection: "company",
        credential_binding: "project-binding-company",
      },
    });
    await page.screenshot({
      path: `output/playwright/credentials-project-${appearance.id}.png`,
      fullPage: true,
    });
    await page.getByRole("button", { name: "Personal", exact: true }).click();
    await sections
      .getByRole("button", { name: "Credentials", exact: true })
      .click();
    const personalInput = page.locator("input[aria-label='API key or token']");
    await page
      .getByRole("button", {
        name: "Add credential",
        exact: true,
      })
      .click();
    await page.screenshot({
      path: `output/playwright/credentials-personal-editor-${appearance.id}.png`,
      fullPage: true,
    });
    await page
      .getByRole("textbox", { name: "Display label", exact: true })
      .fill("Personal provider");
    await page
      .getByRole("combobox", { name: "Credential connection", exact: true })
      .click();
    await page
      .getByRole("option", { name: "Provider: Personal provider", exact: true })
      .click();
    await personalInput.fill("synthetic-private-browser-secret");
    await page
      .getByRole("button", { name: "Save credential", exact: true })
      .click();
    await expect.poll(() => fixture.credentialWrites.length).toBe(2);
    await expect(personalInput).toHaveCount(0);
    await expect(
      page
        .getByRole("region", { name: "Personal credentials", exact: true })
        .getByRole("cell", { name: /Stored securely/ }),
    ).toBeVisible();
    expect(fixture.credentialWrites[1]!.body).toEqual({
      revision: 0,
      label: "Personal provider",
      kind: "api_key",
      secret: "synthetic-private-browser-secret",
      attachment: {
        kind: "provider",
        connection: "personal-key",
        credential_binding: "synthetic-reviewed-binding",
      },
    });
    const browserStorage = await page.evaluate(() =>
      JSON.stringify([localStorage, sessionStorage]),
    );
    expect(browserStorage).not.toContain("synthetic-private-browser-secret");
    expect(browserStorage).not.toContain("synthetic-shared-browser-secret");
    await page.reload();
    await sections
      .getByRole("button", { name: "Credentials", exact: true })
      .click();
    await expect(
      page
        .getByRole("region", { name: "Personal credentials", exact: true })
        .getByRole("cell", { name: /Stored securely/ }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "More actions for Personal provider" })
      .click();
    await page
      .getByRole("menuitem", { name: "Delete Personal provider" })
      .click();
    await page
      .getByRole("button", { name: "Remove credential", exact: true })
      .click();
    await expect(
      page.getByText("Credential changes saved.", { exact: true }),
    ).toBeVisible();
    fixture.verify();
  });
}
