import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

interface FixtureCall {
  command: string;
  args: Record<string, unknown>;
}

interface AlwaysOnFixture {
  calls: FixtureCall[];
  finishWake(): void;
  switchToExternal(): void;
  releaseOldStatus?: () => void;
  releaseOldInventory?: () => void;
}

type FixtureWindow = { alwaysOnFixture: AlwaysOnFixture };

async function installAlwaysOnFixture(
  page: Page,
  options: {
    deferStatus?: boolean;
    deferInventory?: boolean;
    inventory?: boolean;
  } = {},
) {
  await page.addInitScript(({ deferStatus, deferInventory, inventory }) => {
    const host = window as unknown as FixtureWindow & {
      __TAURI_INTERNALS__: unknown;
      __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
    };
    const managed = "always-on-managed";
    const background = "always-on-background";
    const sleeping = "always-on-sleeping";
    const external = "always-on-external";
    const workspace = {
      workspaceId: "always-on-workspace",
      displayName: "Always On Workspace",
      displayPath: "/fixture/always-on",
    };
    const first = "always-on-conversation-1";
    const second = "always-on-conversation-2";
    const firstTime = "2026-11-16T11:00:00Z";
    const nextWake = "2026-11-16T12:30:00Z";
    const requestedAt = "2026-11-16T12:01:00Z";
    const completedAt = "2026-11-16T12:02:00Z";
    const makeRun = (
      id: string,
      sessionId: string,
      title: string,
      createdAt: string,
      output: string,
    ) => ({
      runId: id,
      sessionId,
      title,
      role: "primary",
      mode: "execute",
      status: "completed",
      createdAt,
      updatedAt: createdAt,
      startedAt: createdAt,
      finishedAt: createdAt,
      lastSequence: 0,
      pendingInteractionCount: 0,
      terminal: {
        type: "result",
        result: {
          output,
          profile: "primary",
          modelProfile: "primary",
          providerProfile: "fixture",
          model: "fixture",
          elapsedSeconds: 1,
        },
      },
      etag: id,
      archived: false,
      continuable: true,
    });
    const runs = [
      makeRun("first", first, "Workspace notes", firstTime, "Initial notes."),
      makeRun("second", second, "Release notes", firstTime, "Draft notes."),
    ];
    const backgroundRun = makeRun(
      "background-run",
      "background-conversation",
      "Background tracking",
      firstTime,
      "Background workspace notes.",
    );
    const backgroundProfile = {
      session_id: backgroundRun.sessionId,
      enabled: false,
      revision: 3,
      next_wake_at: nextWake,
      pending: null,
      last_wake: {
        id: "previous",
        completed_at: completedAt,
        status: "completed",
      },
    };
    if (inventory) {
      localStorage.setItem(
        "colossus.thread-pins:v1",
        JSON.stringify([{ spaceId: managed, sessionIds: [first, second] }]),
      );
    }
    const makeSpace = (
      spaceId: string,
      displayName: string,
      state: string,
    ) => ({
      spaceId,
      targetId: spaceId,
      displayName,
      displayPath: `/fixture/${spaceId}`,
      archived: false,
      lastOpenedAtMs: 0,
      lastActivityAt: null,
      state,
      message: "Synthetic runtime ready.",
      selected: selectedTargetId === spaceId,
      attentionCount: 0,
      providerConfigured: true,
    });
    let selectedTargetId = managed;
    let profile: {
      session_id: string;
      enabled: boolean;
      revision: number;
      next_wake_at: string;
      pending: { id: string; requested_at: string; trigger: "manual" } | null;
      last_wake: {
        id: string;
        completed_at: string;
        status: "completed";
      } | null;
    } | null = null;
    const status = () => ({
      releaseChannel: "development",
      connection: {
        state: "connected",
        targetId: selectedTargetId,
        message: "Synthetic connection ready.",
      },
      targets: [
        {
          targetId: selectedTargetId,
          kind:
            selectedTargetId === external ? "external_daemon" : "managed_local",
          label:
            selectedTargetId === managed
              ? workspace.displayName
              : selectedTargetId === background
                ? "Background Workspace"
                : "External fixture",
          state: "ready",
          message: "Synthetic runtime ready.",
          selected: true,
          terminalAvailable: false,
          workspace: selectedTargetId === external ? null : workspace,
          failureCode: null,
        },
      ],
      selectedTargetId,
      spaces: [
        makeSpace(managed, workspace.displayName, "ready"),
        ...(inventory
          ? [
              makeSpace(background, "Background Workspace", "ready"),
              makeSpace(sleeping, "Sleeping Workspace", "sleeping"),
            ]
          : []),
      ],
      selectedSpaceId: selectedTargetId === external ? null : selectedTargetId,
      managedState: "ready",
      workspace: selectedTargetId === external ? null : workspace,
      provider: {
        configured: true,
        kind: "openai_compatible",
        model: "fixture",
      },
      codexAuth: { state: "unavailable", message: "" },
      managedModelConfiguration: { providers: [], models: [], roles: {} },
      accessProfile: "minimal",
      executionBoundary: "offline_isolated",
      approvalMode: "ask",
      terminalEnabled: false,
      additionalCaBundle: {
        configured: false,
        certificateCount: 0,
        fingerprintsSha256: [],
      },
      clientIdentity: { configured: false, leafFingerprintSha256: null },
      capabilities: {
        delegation: false,
        plugins: false,
        tui: false,
        shellTerminal: false,
        files: false,
        artifacts: false,
        planContinuation: false,
        updateAvailable: false,
        agentWorkflows: false,
        attachments: false,
      },
    });
    const fixture: AlwaysOnFixture = {
      calls: [],
      finishWake() {
        if (!profile?.pending)
          throw new Error("Expected a pending fixture wake.");
        runs.unshift(
          makeRun(
            "background-check",
            profile.session_id,
            "Workspace notes",
            completedAt,
            "The background check found one blocked task.",
          ),
        );
        profile = {
          ...profile,
          revision: profile.revision + 1,
          last_wake: {
            id: profile.pending.id,
            completed_at: completedAt,
            status: "completed",
          },
          pending: null,
        };
      },
      switchToExternal() {
        selectedTargetId = external;
      },
    };
    host.alwaysOnFixture = fixture;
    host.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    let callbackId = 0;
    host.__TAURI_INTERNALS__ = {
      transformCallback: () => ++callbackId,
      unregisterCallback: () => {},
      invoke: async (command: string, args: Record<string, unknown> = {}) => {
        if (command === "initialize_desktop" || command === "desktop_status") {
          return status();
        }
        if (command === "plugin:event|listen") return ++callbackId;
        if (command === "browser_context") {
          return {
            available: false,
            generation: 0,
            tabs: [],
            selectedTabId: null,
          };
        }
        if (command === "list_setup_packages" || command === "list_asides")
          return [];
        if (command === "list_desktop_always_on") {
          if (deferInventory) {
            deferInventory = false;
            await new Promise<void>((resolve) => {
              fixture.releaseOldInventory = resolve;
            });
            return [
              {
                spaceId: managed,
                displayName: "Old inventory snapshot",
                state: "enrolled",
                profile: backgroundProfile,
              },
            ];
          }
          return [
            {
              spaceId: managed,
              displayName: workspace.displayName,
              ...(profile
                ? { state: "enrolled", profile }
                : { state: "unenrolled" }),
            },
            ...(inventory
              ? [
                  {
                    spaceId: background,
                    displayName: "Background Workspace",
                    state: "enrolled",
                    profile: backgroundProfile,
                  },
                  {
                    spaceId: sleeping,
                    displayName: "Sleeping Workspace",
                    state: "unavailable",
                  },
                ]
              : []),
          ];
        }
        if (command === "select_space") {
          const spaceId = String(args.spaceId);
          if (!inventory || spaceId !== background)
            throw new Error("Unknown fixture Workspace.");
          fixture.calls.push({ command, args });
          selectedTargetId = spaceId;
          return status();
        }
        if (command === "list_runs") {
          const request = args.request as { sessionId?: string };
          const targetRuns =
            args.targetId === managed
              ? runs
              : args.targetId === background
                ? [backgroundRun]
                : [];
          return {
            runs: targetRuns.filter(
              (run) =>
                !request.sessionId || request.sessionId === run.sessionId,
            ),
            nextPageToken: "",
          };
        }
        if (command === "get_run") {
          const { runId } = args.request as { runId: string };
          const run = [...runs, backgroundRun].find(
            (item) => item.runId === runId,
          );
          if (!run) throw new Error("Unknown fixture run.");
          return { run, pendingInteractions: [] };
        }
        if (!command.startsWith("desktop_always_on_")) return null;
        fixture.calls.push({ command, args });
        if (
          args.targetId === background &&
          selectedTargetId === background &&
          command === "desktop_always_on_status"
        ) {
          return { selectionEpoch: 42, profile: backgroundProfile };
        }
        if (args.targetId !== managed || selectedTargetId !== managed) {
          throw new Error(
            "Fixture received a request for an unavailable target.",
          );
        }
        if (command === "desktop_always_on_status") {
          if (deferStatus) {
            deferStatus = false;
            await new Promise<void>((resolve) => {
              fixture.releaseOldStatus = resolve;
            });
          }
          return { selectionEpoch: 41, profile };
        }
        if (
          args.selectionEpoch !== 41 ||
          args.expectedRevision !== (profile?.revision ?? 0)
        ) {
          throw new Error(
            "Fixture rejected the stale revision or selection epoch.",
          );
        }
        if (command === "desktop_always_on_enable") {
          const sessionId = String(args.sessionId);
          if (
            !runs.some((run) => run.sessionId === sessionId) ||
            (profile?.session_id !== sessionId &&
              (profile?.enabled || profile?.pending))
          ) {
            throw new Error("Fixture rejected the enrolled conversation.");
          }
          profile = {
            session_id: sessionId,
            enabled: true,
            revision: (profile?.revision ?? 0) + 1,
            next_wake_at: nextWake,
            pending: null,
            last_wake: profile?.last_wake ?? null,
          };
        } else if (
          command === "desktop_always_on_wake" &&
          profile?.enabled &&
          !profile.pending
        ) {
          profile = {
            ...profile,
            revision: profile.revision + 1,
            pending: {
              id: "wake-1",
              requested_at: requestedAt,
              trigger: "manual",
            },
          };
        } else if (command === "desktop_always_on_pause" && profile?.enabled) {
          profile = {
            ...profile,
            revision: profile.revision + 1,
            enabled: false,
          };
        } else {
          throw new Error("Unsupported fixture mutation.");
        }
        return profile;
      },
    };
  }, options);
}

async function openThread(page: Page, title: string) {
  await page.getByRole("button", { name: "Open work navigation" }).click();
  await page.getByRole("button", { name: new RegExp(`^${title}`) }).click();
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
}

test("an Always On conversation can wake, pause, and move at compact width", async ({
  page,
}) => {
  await installAlwaysOnFixture(page);
  await page.goto("/");
  await openThread(page, "Workspace notes");
  const trigger = page.getByRole("button", {
    name: "Always On Agent",
    exact: true,
  });
  await trigger.press("Enter");
  const panel = page.getByRole("dialog", { name: "Always On Agent" });
  const enroll = panel.getByRole("button", { name: "Use this conversation" });
  await expect(enroll).toBeEnabled();
  const bounds = await panel.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(880);
  await enroll.press("Enter");
  await expect(panel.getByText("Enabled", { exact: true })).toBeVisible();
  await expect(panel.getByText("Next wake", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  await trigger.press("Enter");
  const wake = panel.getByRole("button", { name: "Wake now" });
  await wake.click();
  await expect(panel.getByText("Manual wake", { exact: true })).toBeVisible();
  await expect(wake).toBeDisabled();
  await page.evaluate(() =>
    (window as unknown as FixtureWindow).alwaysOnFixture.finishWake(),
  );
  await expect(
    page.getByText("The background check found one blocked task."),
  ).toBeVisible();
  await expect(wake).toBeEnabled({ timeout: 10_000 });
  await panel.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(panel.getByText("Paused", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");

  await openThread(page, "Release notes");
  await trigger.press("Enter");
  await expect(
    panel.getByRole("button", { name: "Open conversation" }),
  ).toBeEnabled();
  await panel
    .getByRole("button", { name: "Move to this conversation" })
    .click();
  await expect(
    panel.getByText("This conversation", { exact: true }),
  ).toBeVisible();
  await expect(panel.getByText("Enabled", { exact: true })).toBeVisible();
  const mutations = await page.evaluate(() =>
    (window as unknown as FixtureWindow).alwaysOnFixture.calls.filter(
      (call) => call.command !== "desktop_always_on_status",
    ),
  );
  expect(mutations.map(({ command }) => command)).toEqual([
    "desktop_always_on_enable",
    "desktop_always_on_wake",
    "desktop_always_on_pause",
    "desktop_always_on_enable",
  ]);
  expect(mutations.at(-1)?.args).toEqual({
    targetId: "always-on-managed",
    selectionEpoch: 41,
    expectedRevision: 4,
    sessionId: "always-on-conversation-2",
  });
});

test("a late Always On reply disappears when the selected target becomes external", async ({
  page,
}) => {
  await installAlwaysOnFixture(page, { deferStatus: true });
  await page.goto("/");
  const trigger = page.getByRole("button", {
    name: "Always On Agent",
    exact: true,
  });
  await trigger.click();
  const panel = page.getByRole("dialog", { name: "Always On Agent" });
  await expect(panel.getByRole("status")).toHaveText("Checking status…");
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean(
          (window as unknown as FixtureWindow).alwaysOnFixture.releaseOldStatus,
        ),
      ),
    )
    .toBe(true);
  await page.evaluate(() =>
    (window as unknown as FixtureWindow).alwaysOnFixture.switchToExternal(),
  );
  await expect(trigger).toHaveCount(0, { timeout: 10_000 });
  await page.evaluate(() =>
    (window as unknown as FixtureWindow).alwaysOnFixture.releaseOldStatus?.(),
  );
  await expect(panel).toHaveCount(0);
  await expect(trigger).toHaveCount(0);
});

test("Always On Agents precede Pinned and open across local Workspaces", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1250, height: 780 });
  await installAlwaysOnFixture(page, { inventory: true });
  await page.goto("/");
  const threads = page.getByRole("navigation", {
    name: "Threads",
    exact: true,
  });
  const pinned = threads.getByRole("heading", { name: "Pinned", exact: true });
  await expect(pinned).toBeVisible();
  await expect(
    threads.getByRole("heading", { name: "Always On Agents" }),
  ).toHaveCount(0);
  await threads.getByRole("button", { name: /^Workspace notes/u }).click();
  const trigger = page.getByRole("button", {
    name: "Always On Agent",
    exact: true,
  });
  await trigger.press("Enter");
  const popover = page.getByRole("dialog", { name: "Always On Agent" });
  await popover.getByRole("button", { name: "Use this conversation" }).click();
  await expect(popover.getByText("Enabled", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  const agentsHeading = threads.getByRole("heading", {
    name: "Always On Agents",
  });
  await expect(agentsHeading).toBeVisible();
  expect((await agentsHeading.boundingBox())!.y).toBeLessThan(
    (await pinned.boundingBox())!.y,
  );
  await expect(
    threads.getByRole("button", { name: /^Workspace notes/u }),
  ).toHaveCount(1);
  await expect(
    threads.getByRole("button", {
      name: /^Thread actions for Workspace notes/u,
    }),
  ).toBeAttached();

  const destination = page
    .getByRole("navigation", { name: "Workspace destinations" })
    .getByRole("button", { name: "Always On Agents", exact: true });
  await destination.press("Enter");
  await expect(destination).toHaveAttribute("aria-current", "page");
  const main = page.getByRole("main");
  await expect(
    main.getByRole("heading", { name: "Always On Agents" }),
  ).toBeVisible();
  await expect(
    main.getByText("2 agents · 1 Workspace unavailable"),
  ).toBeVisible();
  const agents = main.getByRole("list", { name: "Enrolled agents" });
  await expect(agents.getByText("Enabled", { exact: true })).toBeVisible();
  await expect(agents.getByText("Paused", { exact: true })).toBeVisible();
  await expect(agents.getByText(/Last wake completed/)).toBeVisible();
  const unavailable = main.getByRole("region", {
    name: "Unavailable Workspaces",
  });
  await expect(
    unavailable.getByText("Sleeping Workspace", { exact: true }),
  ).toBeVisible();
  await expect(
    unavailable.getByRole("button", {
      name: "Open Workspace Sleeping Workspace",
    }),
  ).toBeEnabled();
  expect(
    await page.evaluate(() =>
      (window as unknown as FixtureWindow).alwaysOnFixture.calls.filter(
        (call) => call.command === "select_space",
      ),
    ),
  ).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath("desktop-inventory.png") });

  await agents
    .getByRole("button", { name: "Open conversation in Background Workspace" })
    .press("Enter");
  await expect(
    main.getByRole("heading", { name: "Background tracking" }),
  ).toBeVisible();
  await trigger.press("Enter");
  await expect(
    popover.getByRole("button", { name: "Resume", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(() =>
      (window as unknown as FixtureWindow).alwaysOnFixture.calls
        .filter((call) => call.command === "select_space")
        .map((call) => call.args.spaceId),
    ),
  ).toEqual(["always-on-background"]);
});

test("Always On inventory fits Large text and keyboard navigation across palettes", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 390, height: 720 });
  await installAlwaysOnFixture(page, { inventory: true });
  const variants = [
    { colorTheme: "light", darkPalette: "colossus", forcedColors: "none" },
    { colorTheme: "dark", darkPalette: "colossus", forcedColors: "none" },
    { colorTheme: "dark", darkPalette: "neutral", forcedColors: "none" },
    { colorTheme: "dark", darkPalette: "black", forcedColors: "none" },
    { colorTheme: "dark", darkPalette: "hacker", forcedColors: "none" },
    { colorTheme: "dark", darkPalette: "colossus", forcedColors: "active" },
  ] as const;
  test.setTimeout(90_000);
  await page.goto("/");
  for (const variant of variants) {
    const { forcedColors, ...preference } = variant;
    await page.emulateMedia({ forcedColors });
    await page.evaluate((appearance) => {
      localStorage.setItem(
        "colossus.desktop.appearance.v1",
        JSON.stringify({ ...appearance, textSize: "large" }),
      );
    }, preference);
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-theme",
      variant.colorTheme,
    );
    if (variant.colorTheme === "dark") {
      await expect(page.locator("html")).toHaveAttribute(
        "data-palette",
        variant.darkPalette,
      );
    }
    await page
      .getByRole("button", { name: "Open work navigation" })
      .press("Enter");
    await page
      .getByRole("navigation", { name: "Workspace destinations" })
      .getByRole("button", { name: "Always On Agents", exact: true })
      .press("Enter");
    const main = page.getByRole("main");
    await expect(
      main.getByRole("heading", { name: "Always On Agents" }),
    ).toBeVisible();
    await expect(main).toBeFocused();
    const conversation = main.getByRole("button", {
      name: "Open conversation in Background Workspace",
    });
    await expect(conversation).toBeVisible();
    for (const element of [
      main.getByRole("heading", { name: "Always On Agents" }),
      main.getByRole("button", { name: "Refresh", exact: true }),
      conversation,
    ]) {
      const bounds = (await element.boundingBox())!;
      expect(bounds.x).toBeGreaterThanOrEqual(0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(390);
    }
    await conversation.focus();
    await expect(conversation).toBeFocused();
    await page.keyboard.press("Tab");
    const sleepingWorkspace = main.getByRole("button", {
      name: "Open Workspace Sleeping Workspace",
    });
    await expect(sleepingWorkspace).toBeFocused();
    if (variant.colorTheme === "light" || variant.darkPalette === "hacker") {
      const result = await new AxeBuilder({ page })
        .include("#primary-workspace")
        .analyze();
      expect(
        result.violations.filter(
          ({ impact }) => impact === "critical" || impact === "serious",
        ),
      ).toEqual([]);
      await page.screenshot({
        path: testInfo.outputPath(
          `compact-${variant.colorTheme}-${variant.darkPalette}.png`,
        ),
      });
    }
  }
});

test("an earlier inventory response is discarded after the selected runtime changes", async ({
  page,
}) => {
  await installAlwaysOnFixture(page, { inventory: true, deferInventory: true });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Open work navigation" })
    .press("Enter");
  await page
    .getByRole("navigation", { name: "Workspace destinations" })
    .getByRole("button", { name: "Always On Agents", exact: true })
    .press("Enter");
  const main = page.getByRole("main");
  await expect(main.getByRole("status")).toHaveText(
    "Checking Always On Agents…",
  );
  await page.evaluate(() =>
    (window as unknown as FixtureWindow).alwaysOnFixture.switchToExternal(),
  );
  await expect(
    main.getByRole("heading", { name: "Background Workspace" }),
  ).toBeVisible();
  await page.evaluate(async () => {
    (
      window as unknown as FixtureWindow
    ).alwaysOnFixture.releaseOldInventory?.();
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
  });
  await expect(
    main.getByRole("heading", { name: "Background Workspace" }),
  ).toBeVisible();
  await expect(main.getByText("Old inventory snapshot")).toHaveCount(0);
});
