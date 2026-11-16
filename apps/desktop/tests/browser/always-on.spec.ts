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
}

type FixtureWindow = { alwaysOnFixture: AlwaysOnFixture };

async function installAlwaysOnFixture(page: Page, deferStatus = false) {
  await page.addInitScript((deferStatus) => {
    const host = window as unknown as FixtureWindow & {
      __TAURI_INTERNALS__: unknown;
      __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
    };
    const managed = "always-on-managed";
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
            selectedTargetId === managed ? "managed_local" : "external_daemon",
          label:
            selectedTargetId === managed
              ? workspace.displayName
              : "External fixture",
          state: "ready",
          message: "Synthetic runtime ready.",
          selected: true,
          terminalAvailable: false,
          workspace: selectedTargetId === managed ? workspace : null,
          failureCode: null,
        },
      ],
      selectedTargetId,
      spaces: [
        {
          spaceId: managed,
          targetId: managed,
          displayName: workspace.displayName,
          displayPath: workspace.displayPath,
          archived: false,
          lastOpenedAtMs: 0,
          lastActivityAt: null,
          state: "ready",
          message: "Synthetic runtime ready.",
          selected: selectedTargetId === managed,
          attentionCount: 0,
          providerConfigured: true,
        },
      ],
      selectedSpaceId: selectedTargetId === managed ? managed : null,
      managedState: "ready",
      workspace: selectedTargetId === managed ? workspace : null,
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
        if (command === "list_runs") {
          const request = args.request as { sessionId?: string };
          return {
            runs:
              args.targetId === managed
                ? runs.filter(
                    (run) =>
                      !request.sessionId || request.sessionId === run.sessionId,
                  )
                : [],
            nextPageToken: "",
          };
        }
        if (command === "get_run") {
          const { runId } = args.request as { runId: string };
          const run = runs.find((item) => item.runId === runId);
          if (!run) throw new Error("Unknown fixture run.");
          return { run, pendingInteractions: [] };
        }
        if (!command.startsWith("desktop_always_on_")) return null;
        fixture.calls.push({ command, args });
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
  }, deferStatus);
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
  const trigger = page.getByRole("button", { name: "Always On Agent" });
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
  await installAlwaysOnFixture(page, true);
  await page.goto("/");
  const trigger = page.getByRole("button", { name: "Always On Agent" });
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
