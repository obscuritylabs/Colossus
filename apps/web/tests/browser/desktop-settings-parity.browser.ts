import { expect, test, type Page } from "@playwright/test";
import type { ProviderPreset } from "@colossus/ui/lib/provider-catalog";
import type { ChatSettingsSnapshot } from "../../src/chat-settings";
import { browserFixture } from "./fixture";
import {
  appearanceCases,
  openAppSidebar,
  openConversationList,
  selectAppearance,
} from "./appearance";

// Exercise every canonical provider through the same host-supplied shared picker.
const providerPresets: ProviderPreset[] = [
  {
    id: "codex",
    label: "Codex / ChatGPT",
    protocol: "codex",
    baseUrl: "https://chatgpt.com/backend-api/codex",
    credentialEnv: null,
  },
  {
    id: "openai",
    label: "OpenAI",
    protocol: "responses",
    baseUrl: "https://api.openai.com/v1",
    credentialEnv: "OPENAI_API_KEY",
  },
  {
    id: "anthropic",
    label: "Anthropic",
    protocol: "anthropic_messages",
    baseUrl: "https://api.anthropic.com/v1",
    credentialEnv: "ANTHROPIC_API_KEY",
  },
  ...[
    [
      "openrouter",
      "OpenRouter",
      "https://openrouter.ai/api/v1",
      "OPENROUTER_API_KEY",
    ],
    ["groq", "Groq", "https://api.groq.com/openai/v1", "GROQ_API_KEY"],
    [
      "together",
      "Together AI",
      "https://api.together.xyz/v1",
      "TOGETHER_API_KEY",
    ],
    ["deepseek", "DeepSeek", "https://api.deepseek.com/v1", "DEEPSEEK_API_KEY"],
    ["mistral", "Mistral", "https://api.mistral.ai/v1", "MISTRAL_API_KEY"],
    ["ollama", "Ollama", "http://localhost:11434/v1", null],
    ["lmstudio", "LM Studio", "http://localhost:1234/v1", null],
  ].map(([id, label, baseUrl, credentialEnv]) => ({
    id: id!,
    label: label!,
    protocol: "chat_completions" as const,
    baseUrl: baseUrl!,
    credentialEnv: credentialEnv ?? null,
  })),
  {
    id: "custom-chat",
    label: "Custom Chat Completions",
    protocol: "chat_completions",
    baseUrl: null,
    credentialEnv: null,
  },
  {
    id: "custom-responses",
    label: "Custom Responses",
    protocol: "responses",
    baseUrl: null,
    credentialEnv: null,
  },
];

const modelSettings: Partial<ChatSettingsSnapshot> = {
  models: {
    profiles: {
      careful: {
        providerProfile: "company",
        model: "configured-model",
        contextWindowTokens: 128000,
        maxOutputTokens: 16000,
        capabilities: {
          toolCalls: "auto",
          streaming: "auto",
          imageInputs: "auto",
          serverCompaction: "auto",
        },
        reasoningEffort: "high",
      },
      fast: {
        providerProfile: "company",
        model: "fast-model",
        contextWindowTokens: 64000,
        maxOutputTokens: 8000,
        capabilities: {
          toolCalls: "auto",
          streaming: "auto",
          imageInputs: "auto",
          serverCompaction: "auto",
        },
        reasoningEffort: null,
      },
    },
    roles: { primary: "careful" },
  },
  labels: {
    providers: { company: "Company AI" },
    models: { careful: "Careful", fast: "Fast" },
  },
};

async function openProjectSettings(page: Page) {
  await openAppSidebar(page);
  await page.getByRole("link", { name: "Chat", exact: true }).click();
  await openConversationList(page);
  await page.getByRole("button", { name: "Chat options", exact: true }).click();
  await page
    .getByRole("menuitem", { name: "Project settings", exact: true })
    .click();
}

async function chooseSettingsSection(page: Page, name: string) {
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name, exact: true })
    .click();
}

async function assertSettingsBounds(page: Page) {
  const bounds = await page
    .locator(
      ".web-settings-window, .settings-main, .model-role-routing, .provider-editor, .model-editor",
    )
    .evaluateAll((elements) =>
      elements
        .filter((element) => element.getClientRects().length)
        .map((element) => ({
          name: element.className,
          left: element.getBoundingClientRect().left,
          right: element.getBoundingClientRect().right,
          client: element.clientWidth,
          scroll: element.scrollWidth,
          viewport: innerWidth,
        })),
    );
  for (const bound of bounds) {
    expect(bound.left, JSON.stringify(bound)).toBeGreaterThanOrEqual(0);
    expect(bound.right, JSON.stringify(bound)).toBeLessThanOrEqual(
      bound.viewport + 1,
    );
    expect(bound.scroll, JSON.stringify(bound)).toBeLessThanOrEqual(
      bound.client + 1,
    );
  }
}

test("provider inventory uses the full Desktop picker and persists provider display identity", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  const fixture = await browserFixture(page, {
    administer: true,
    settingsOverride: modelSettings,
    providerPresets,
  });
  await selectAppearance(page, appearanceCases[0]);
  await openProjectSettings(page);
  const inventory = page.getByRole("table", {
    name: "Configured providers",
    exact: true,
  });
  await expect(inventory).toBeVisible();
  await expect(inventory.locator(".catalog-inventory-row")).toHaveCount(1);
  await expect(inventory).toContainText("Company AI");
  await page
    .getByRole("textbox", { name: "Search providers", exact: true })
    .fill("missing provider");
  await expect(inventory).toContainText("No matching providers");
  await page.getByRole("button", { name: "Clear search", exact: true }).click();
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  const editor = page.locator(".provider-editor");
  await expect(
    editor.getByRole("heading", { name: "Identity", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("heading", { name: "Connection", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("heading", { name: "Authentication", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("heading", { name: "Runtime", exact: true }),
  ).toBeVisible();
  const provider = editor.getByRole("combobox", {
    name: "Provider",
    exact: true,
  });
  await provider.click();
  const picker = page.getByRole("listbox", { name: "Provider", exact: true });
  for (const preset of providerPresets) {
    await expect(
      picker.getByRole("option", { name: preset.label, exact: true }),
    ).toBeAttached();
  }
  await picker
    .getByRole("option", { name: "Codex / ChatGPT", exact: true })
    .click();
  await expect(editor).toContainText(
    "Codex sign-in is managed by your deployment administrator.",
  );
  await expect(
    editor.getByRole("textbox", { name: /^Endpoint URL/u }),
  ).toBeDisabled();
  expect(fixture.settingsSubmissions).toHaveLength(0);
  await provider.click();
  await picker.getByRole("option", { name: "OpenRouter", exact: true }).click();
  await expect(
    provider.locator('[data-provider-brand="openrouter"]'),
  ).toBeVisible();
  await expect(
    editor.getByRole("textbox", { name: /^Endpoint URL/u }),
  ).toHaveValue("https://openrouter.ai/api/v1");
  await editor
    .getByRole("textbox", { name: /^Display label/u })
    .fill("Team OpenRouter");
  await editor
    .getByRole("textbox", { name: /^Profile ID/u })
    .fill("team-router");
  await editor
    .getByRole("button", { name: "Add provider", exact: true })
    .click();
  await expect.poll(() => fixture.settingsSubmissions.length).toBe(1);
  expect(fixture.settingsSubmissions[0]).toMatchObject({
    labels: {
      providers: { company: "Company AI", "team-router": "Team OpenRouter" },
    },
    providers: {
      profiles: {
        "team-router": {
          kind: "openai_compatible",
          baseUrl: "https://openrouter.ai/api/v1",
        },
      },
    },
  });
  await expect(inventory).toContainText("Team OpenRouter");
  await page.reload();
  await page.getByRole("button", { name: "Project", exact: true }).click();
  await chooseSettingsSection(page, "Providers");
  await expect(
    page.getByRole("table", { name: "Configured providers", exact: true }),
  ).toContainText("Team OpenRouter");
  await page.screenshot({
    path: "output/playwright/desktop-provider-inventory-web.png",
  });
  fixture.verify();
});

test("Desktop model cards preserve automatic capabilities and the shared model editor layout", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  const fixture = await browserFixture(page, {
    administer: true,
    settingsOverride: modelSettings,
    providerPresets,
  });
  await selectAppearance(page, appearanceCases[0]);
  await openProjectSettings(page);
  await chooseSettingsSection(page, "Models");
  await expect(
    page.getByRole("table", { name: "Configured models", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Edit careful", exact: true }).click();
  const editor = page.locator(".model-editor");
  await expect(
    editor.getByRole("heading", { name: "Identity", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("heading", { name: "Provider connection", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("heading", { name: "Token limits", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("heading", { name: "Supported features", exact: true }),
  ).toBeVisible();
  await expect(
    editor.getByRole("textbox", { name: /^Display label/u }),
  ).toHaveValue("Careful");
  await editor
    .getByRole("textbox", { name: /^Display label/u })
    .fill("Catalog reasoning");
  for (const label of [
    "Tool calls",
    "Streaming",
    "Image inputs",
    "Server compaction",
  ]) {
    await expect(
      editor.getByRole("slider", { name: label, exact: true }),
    ).toHaveAttribute("aria-valuetext", "Auto");
  }
  await editor
    .getByRole("button", { name: "Load models", exact: true })
    .click();
  await editor
    .getByRole("searchbox", { name: "Search models", exact: true })
    .fill("Catalog Reasoner");
  await editor.getByRole("button", { name: /^Catalog Reasoner/ }).click();
  await expect(
    editor.getByRole("textbox", { name: /^Model identifier/u }),
  ).toHaveValue("catalog-reasoner");
  await expect(
    editor.getByRole("spinbutton", {
      name: /^Context window \(tokens\)/u,
    }),
  ).toHaveValue("200000");
  const output = editor.getByRole("spinbutton", {
    name: /^Maximum output \(tokens\)/u,
  });
  await expect(output).toHaveValue("30000");
  await output.fill("12000");
  await editor
    .getByRole("button", { name: "Refresh models", exact: true })
    .click();
  await expect(output).toHaveValue("12000");
  for (const label of [
    "Tool calls",
    "Streaming",
    "Image inputs",
    "Server compaction",
  ]) {
    const control = editor.getByRole("group", { name: label, exact: true });
    await expect(control).toContainText("Model card reports support.");
    await expect(
      control.getByRole("slider", { name: label, exact: true }),
    ).toHaveAttribute("aria-valuetext", "Auto");
  }
  await page.screenshot({
    path: "output/playwright/desktop-model-editor-web.png",
  });
  await editor
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect.poll(() => fixture.settingsSubmissions.length).toBe(1);
  expect(fixture.settingsSubmissions[0]).toMatchObject({
    labels: { models: { careful: "Catalog reasoning" } },
    models: {
      profiles: {
        careful: {
          model: "catalog-reasoner",
          maxOutputTokens: 12000,
          capabilities: {
            toolCalls: "auto",
            streaming: "auto",
            imageInputs: "auto",
            serverCompaction: "auto",
            declared: {
              toolCalls: true,
              streaming: true,
              imageInputs: true,
              serverCompaction: true,
            },
          },
        },
      },
    },
  });
  fixture.verify();
});

test("role routing inherits Primary and persists only explicit assignments", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  const fixture = await browserFixture(page, {
    administer: true,
    settingsOverride: modelSettings,
  });
  await selectAppearance(page, appearanceCases[0]);
  await openProjectSettings(page);
  await chooseSettingsSection(page, "Role routing");
  const routing = page.getByRole("region", {
    name: "Role routing",
    exact: true,
  });
  await expect(routing.getByRole("combobox")).toHaveCount(7);
  const primary = routing.getByRole("combobox", {
    name: "Primary model",
    exact: true,
  });
  const planner = routing.getByRole("combobox", {
    name: "Research planner model",
    exact: true,
  });
  await expect(primary).toContainText("Careful · configured-model");
  await expect(planner).toHaveText("Use primary");
  await expect(
    routing.getByText("Uses configured-model · company", { exact: true }),
  ).toHaveCount(7);
  await planner.click();
  await page
    .getByRole("option", { name: "Fast · fast-model", exact: true })
    .click();
  await expect(
    routing.getByText("Uses configured-model · company", { exact: true }),
  ).toHaveCount(6);
  await page
    .getByRole("button", { name: "Save project configuration", exact: true })
    .click();
  await expect.poll(() => fixture.settingsSubmissions.length).toBe(1);
  expect(fixture.settingsSubmissions[0]!.models).toMatchObject({
    roles: { primary: "careful", research_planner: "fast" },
  });
  expect(
    (fixture.settingsSubmissions[0]!.models as { roles: unknown }).roles,
  ).toEqual({ primary: "careful", research_planner: "fast" });
  await primary.click();
  await expect(
    page.getByRole("option", { name: "Use primary", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("option", { name: "Fast · fast-model", exact: true })
    .click();
  await expect(planner).toHaveText("Fast · fast-model");
  await expect(
    routing.getByText("Uses fast-model · company", { exact: true }),
  ).toHaveCount(7);
  await planner.click();
  await page.getByRole("option", { name: "Use primary", exact: true }).click();
  await page
    .getByRole("button", { name: "Save project configuration", exact: true })
    .click();
  await expect.poll(() => fixture.settingsSubmissions.length).toBe(2);
  expect(
    (fixture.settingsSubmissions[1]!.models as { roles: unknown }).roles,
  ).toEqual({ primary: "fast" });
  await page
    .locator(".model-routing-overview")
    .getByText("Routing map", { exact: true })
    .click();
  await expect(page.locator(".model-routing-overview")).toContainText(
    "fast-model",
  );
  await page.screenshot({
    path: "output/playwright/desktop-model-routing-web.png",
  });
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`Desktop settings controls fit narrow Large text in ${appearance.label}`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      administer: true,
      settingsOverride: modelSettings,
      providerPresets,
    });
    await selectAppearance(page, appearance);
    await openProjectSettings(page);
    await page
      .getByRole("button", { name: "Edit company", exact: true })
      .click();
    await assertSettingsBounds(page);
    await page
      .locator(".provider-editor")
      .getByRole("textbox", { name: /^Endpoint URL/u })
      .focus();
    await expect(
      page
        .locator(".provider-editor")
        .getByRole("textbox", { name: /^Endpoint URL/u }),
    ).toBeInViewport();
    await page
      .locator(".provider-editor")
      .getByRole("button", { name: "Cancel", exact: true })
      .click();
    await chooseSettingsSection(page, "Models");
    await page
      .getByRole("button", { name: "Edit careful", exact: true })
      .click();
    await assertSettingsBounds(page);
    const compaction = page
      .locator(".model-editor")
      .getByRole("slider", { name: "Server compaction", exact: true });
    await compaction.focus();
    await compaction.press("End");
    await expect(compaction).toHaveAttribute("aria-valuetext", "On");
    await compaction.press("ArrowLeft");
    await expect(compaction).toHaveAttribute("aria-valuetext", "Auto");
    await page.screenshot({
      path: `output/playwright/desktop-model-editor-narrow-${appearance.id}.png`,
    });
    await page
      .locator(".model-editor")
      .getByRole("button", { name: "Cancel", exact: true })
      .click();
    await chooseSettingsSection(page, "Role routing");
    await assertSettingsBounds(page);
    const synth = page.getByRole("combobox", {
      name: "Research synthesizer model",
      exact: true,
    });
    await synth.focus();
    await synth.press("Enter");
    await page
      .getByRole("option", { name: "Fast · fast-model", exact: true })
      .click();
    await expect(synth).toBeInViewport();
    await page.screenshot({
      path: `output/playwright/desktop-model-routing-narrow-${appearance.id}.png`,
    });
    fixture.verify();
  });
}
