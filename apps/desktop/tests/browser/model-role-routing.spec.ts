import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function openRouting(page: Page, unconfiguredModel = false) {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/?fixture=operations-studio");
  await page.evaluate(async (unconfiguredModel) => {
    const modulePath = "/src/components/ManagedSettingsPane.tsx";
    const { buildManagedSettingsFixture } = await import(modulePath);
    const snapshot = buildManagedSettingsFixture({
      selectedSpaceId: "fixture-managed-local",
      spaces: [
        {
          spaceId: "fixture-managed-local",
          displayName: "Colossus",
          displayPath: "~/Colossus",
          archived: false,
        },
      ],
      managedModelConfiguration: {
        providers: [
          {
            profile: "company",
            providerKind: "openai_compatible",
            baseUrl: "https://ai.example.test/v1",
            timeoutMs: null,
            hasCredential: false,
          },
        ],
        models: [
          {
            profile: "general",
            providerProfile: "company",
            model: "general-model",
            contextWindowTokens: 32000,
            maxOutputTokens: 4000,
            capabilities: {
              toolCalls: true,
              streaming: true,
              imageInputs: false,
            },
            reasoningEffort: null,
          },
          {
            profile: "fast",
            providerProfile: "company",
            model: "fast-model",
            contextWindowTokens: 32000,
            maxOutputTokens: 4000,
            capabilities: {
              toolCalls: true,
              streaming: true,
              imageInputs: false,
            },
            reasoningEffort: null,
          },
        ],
        roles: { primary: "general" },
      },
      accessProfile: "development",
      executionBoundary: "workspace_isolated",
    });
    const space = snapshot.spaces[0];
    space.pendingGlobalRevision = null;
    space.status = "active";
    space.configuration.acceptedGlobalRevision =
      snapshot.globalConfiguration.revision;
    // Older workspaces keep their routes outside the sparse configuration record.
    space.configuration.modelRoles = {};
    space.effectiveModelRoles = { primary: "general" };
    if (unconfiguredModel) {
      const fast = snapshot.globalConfiguration.models.find(
        (entry: any) => entry.revisions[0].value.profile === "fast",
      );
      for (const [key, reference] of Object.entries(
        space.configuration.catalogRevisions,
      )) {
        if ((reference as any).resourceId === fast.id)
          delete space.configuration.catalogRevisions[key];
      }
    }
    const host = window as unknown as Record<string, any>;
    host.roleRequests = [];
    host.profileTestRequests = [];
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any) => {
        if (command === "list_setup_packages") return [];
        if (command === "get_managed_configuration")
          return structuredClone(snapshot);
        if (command === "sync_managed_configuration") return null;
        if (
          command === "diagnose_managed_provider" ||
          command === "diagnose_managed_model"
        ) {
          host.profileTestRequests.push({ command, request: args.request });
          return new Promise((resolve, reject) => {
            host.finishProfileTest = (ready: boolean) =>
              resolve({
                kind:
                  command === "diagnose_managed_provider"
                    ? "provider"
                    : "model",
                profile: args.request.profile,
                ready,
                checks: [
                  {
                    name: "connection",
                    status: ready ? "pass" : "fail",
                    detail: ready
                      ? "Endpoint responded successfully."
                      : "The configured model was not found.",
                  },
                ],
                resultCount: null,
              });
            host.rejectProfileTest = () =>
              reject({
                code: "unavailable",
                message: "Connection timed out. Retry the test.",
                retryable: true,
                outcomeUnknown: false,
                violations: [],
              });
          });
        }
        if (command === "save_space_configuration") {
          host.roleRequests.push(args.request);
          space.configuration.modelRoles = args.request.modelRoles;
          space.effectiveModelRoles = args.request.modelRoles;
          return structuredClone(snapshot);
        }
        throw new Error(`Unexpected routing test command: ${command}`);
      },
    };
  }, unconfiguredModel);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Providers", exact: true })
    .click();
  return page.getByRole("region", { name: "Role routing" });
}

test("provider and model tests stay in their rows through progress, failure, and retry", async ({
  page,
}, testInfo) => {
  await openRouting(page);
  const provider = page.getByRole("group", {
    name: "Provider company",
    exact: true,
  });
  const general = page.getByRole("group", {
    name: "Model general",
    exact: true,
  });
  const fast = page.getByRole("group", { name: "Model fast", exact: true });
  await expect(
    provider.getByRole("button", { name: "Test provider company" }),
  ).toBeEnabled();
  await expect(
    general.getByRole("button", { name: "Test model general" }),
  ).toBeEnabled();
  await expect(
    fast.getByRole("button", { name: "Test model fast" }),
  ).toBeEnabled();
  await expect(page.locator(".managed-diagnostic-actions")).toHaveCount(0);
  await provider.getByRole("button", { name: "Test provider company" }).click();
  await expect(provider.getByRole("status")).toHaveText("Testing company…");
  await expect(provider.getByRole("button")).toBeDisabled();
  await expect(provider.getByRole("switch")).toBeChecked();
  await page.evaluate(() => (window as any).finishProfileTest(true));
  await expect(
    provider.getByText("Test passed", { exact: true }),
  ).toBeVisible();
  await expect(general.getByText("Test passed", { exact: true })).toHaveCount(
    0,
  );
  await provider.getByText("Test details", { exact: true }).click();
  await expect(
    provider.getByText("Endpoint responded successfully."),
  ).toBeVisible();
  await general.getByRole("button", { name: "Test model general" }).click();
  await page.evaluate(() => (window as any).rejectProfileTest());
  await expect(general.getByRole("alert")).toContainText(
    "Connection timed out",
  );
  await expect(
    provider.getByText("Test passed", { exact: true }),
  ).toBeVisible();
  await general.getByRole("button", { name: "Test model general" }).click();
  await expect(general.getByRole("alert")).toHaveCount(0);
  await page.evaluate(() => (window as any).finishProfileTest(false));
  await expect(
    general.getByText("The configured model was not found."),
  ).toBeVisible();
  await general.getByRole("button", { name: "Test model general" }).click();
  await page.evaluate(() => (window as any).finishProfileTest(true));
  await expect(general.getByText("Test passed", { exact: true })).toBeVisible();
  await expect(general.getByRole("switch")).toBeChecked();
  expect(
    await page.evaluate(() => (window as any).profileTestRequests),
  ).toEqual([
    {
      command: "diagnose_managed_provider",
      request: { spaceId: "fixture-managed-local", profile: "company" },
    },
    ...Array.from({ length: 3 }, () => ({
      command: "diagnose_managed_model",
      request: { spaceId: "fixture-managed-local", profile: "general" },
    })),
  ]);
  await provider.getByText("Test details", { exact: true }).click();
  await page
    .locator(".settings-main")
    .evaluate((element) => element.scrollTo(0, 0));
  await page.screenshot({
    path: testInfo.outputPath("provider-model-row-tests.png"),
  });
  await page.setViewportSize({ width: 700, height: 900 });
  await general.getByRole("button", { name: "Test model general" }).focus();
  await expect(
    general.getByRole("button", { name: "Test model general" }),
  ).toBeInViewport();
  expect(
    (
      await new AxeBuilder({ page })
        .include(".managed-profile-resource")
        .analyze()
    ).violations,
  ).toEqual([]);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
});

test("newly selected models must be applied before testing", async ({
  page,
}) => {
  await openRouting(page, true);
  const fast = page.getByRole("group", { name: "Model fast", exact: true });
  await expect(
    fast.getByRole("button", { name: "Test model fast" }),
  ).toBeDisabled();
  await fast.getByRole("switch").click();
  await expect(
    fast.getByText("Apply workspace changes before testing."),
  ).toBeVisible();
  await expect(
    fast.getByRole("button", { name: "Test model fast" }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() => (window as any).profileTestRequests),
  ).toEqual([]);
});

test("all seven routes show their purpose and persist optional research assignments with primary fallback", async ({
  page,
}) => {
  const routing = await openRouting(page);
  await expect(routing.getByRole("combobox")).toHaveCount(7);
  await expect(
    routing.getByRole("combobox", { name: "Primary model", exact: true }),
  ).toHaveText("general · general-model");
  const planner = routing.getByRole("combobox", {
    name: "Research planner model",
    exact: true,
  });
  await expect(planner).toHaveText("Use primary");
  await expect(routing).toContainText(
    "Reads collected sources and extracts factual claims for the research question.",
  );
  await expect(routing).toContainText(
    "Combines source-backed findings into the final research report, with citations and limitations.",
  );
  await expect(
    routing.getByText("Uses general-model · company", { exact: true }),
  ).toHaveCount(7);
  await planner.click();
  await page
    .getByRole("option", { name: "fast · fast-model", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Apply Workspace changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).roleRequests.at(-1)?.modelRoles),
    )
    .toEqual({ primary: "general", research_planner: "fast" });
  // Changing primary updates every inherited route, but keeps explicit assignments.
  await routing
    .getByRole("combobox", { name: "Primary model", exact: true })
    .click();
  await expect(
    page.getByRole("option", { name: "Use primary", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("option", { name: "fast · fast-model", exact: true })
    .click();
  await expect(
    routing.getByText("Uses fast-model · company", { exact: true }),
  ).toHaveCount(7);
  await planner.click();
  await page.getByRole("option", { name: "Use primary", exact: true }).click();
  await page
    .getByRole("button", { name: "Apply Workspace changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).roleRequests.at(-1)?.modelRoles),
    )
    .toEqual({ primary: "fast" });
  // Removing primary must expose an explicit choice, not an apparent self-fallback.
  await page.getByRole("switch", { name: "Select fast", exact: true }).click();
  await expect(
    routing.getByRole("combobox", { name: "Primary model", exact: true }),
  ).toHaveText("Choose a model");
  await expect(
    routing.getByText("Choose a primary model.", { exact: true }),
  ).toHaveCount(7);
});

for (const colorScheme of ["light", "dark"] as const) {
  test(`routing is accessible and fits narrow layouts in ${colorScheme}`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme });
    const routing = await openRouting(page);
    await routing.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: testInfo.outputPath(`model-role-routing-${colorScheme}.png`),
    });
    expect(
      (await new AxeBuilder({ page }).include(".model-role-routing").analyze())
        .violations,
    ).toEqual([]);
    await page.setViewportSize({ width: 700, height: 900 });
    await routing
      .getByRole("combobox", {
        name: "Research synthesizer model",
        exact: true,
      })
      .focus();
    await page.keyboard.press("Enter");
    await page
      .getByRole("option", { name: "fast · fast-model", exact: true })
      .click();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await expect(
      routing.getByRole("combobox", {
        name: "Research synthesizer model",
        exact: true,
      }),
    ).toBeInViewport();
  });
}
