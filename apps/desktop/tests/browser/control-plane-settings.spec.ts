import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("uncertain sharing requires restart instead of another mutation", async ({
  page,
}) => {
  await page.goto("/?fixture=cloud&state=disconnected&recovery=1&restart=1");
  const notice = page
    .getByRole("alert")
    .filter({ hasText: "Sharing needs reconciliation" });
  await expect(notice).toContainText(
    "Restart Desktop before changing sharing or reconnecting.",
  );
  await expect(notice).not.toContainText("Save your choice again");
  for (const name of [
    "Reconnect runtime",
    "Revoke enrollment",
    "Forget enrollment",
    "Save conversation sharing",
  ]) {
    await expect(
      page.getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  }
  await expect(
    page.getByRole("radio", {
      name: "Control Plane conversations only",
      exact: true,
    }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await expect(page.locator(".control-plane-workspace-list")).toContainText(
    "Restart Desktop to reconcile sharing",
  );
});

test("sharing recovery stays visible until an explicit saved choice succeeds", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 850 });
  await page.goto("/?fixture=cloud&state=disconnected&recovery=1");
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
    document.documentElement.dataset.palette = "neutral";
    document.documentElement.dataset.textSize = "large";
    const host = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (command: string, parameters: unknown) => Promise<unknown>;
      };
      sharingCalls: unknown[];
    };
    host.sharingCalls = [];
    const invoke = host.__TAURI_INTERNALS__.invoke;
    host.__TAURI_INTERNALS__.invoke = (command, parameters) => {
      if (
        command === "cloud_set_workspace_sharing" ||
        command === "cloud_connect"
      )
        host.sharingCalls.push({ command, parameters });
      return invoke(command, parameters);
    };
  });
  const notice = page.getByRole("alert").filter({
    hasText: "Sharing needs reconciliation",
  });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("Synchronization is paused");
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await expect(page.locator(".control-plane-workspace-list")).toContainText(
    "Sharing needs reconciliation",
  );
  await expect(page.locator(".control-plane-workspace-list")).not.toContainText(
    "Local history private",
  );
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  await expect(notice).toBeVisible();
  await page
    .getByRole("radio", { name: "Share Desktop history for viewing" })
    .click();
  expect(
    await page.evaluate(
      () => (window as unknown as { sharingCalls: unknown[] }).sharingCalls,
    ),
  ).toEqual([]);
  await expect(notice).toBeVisible();
  const result = await new AxeBuilder({ page })
    .include(".managed-settings-shell")
    .analyze();
  expect(result.violations).toEqual([]);
  expect(
    await page
      .locator(".cloud-settings")
      .evaluate((body) => body.scrollWidth <= body.clientWidth),
  ).toBe(true);
  await page.screenshot({
    path: "output/playwright/control-plane-sharing-recovery.png",
  });
  await page.getByRole("button", { name: "Save conversation sharing" }).click();
  await expect(notice).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as unknown as { sharingCalls: unknown[] }).sharingCalls,
    ),
  ).toEqual([
    {
      command: "cloud_set_workspace_sharing",
      parameters: {
        targetId: "preview-workspace",
        enabled: true,
        allowContinuation: false,
      },
    },
  ]);
});

test("workspace Control Plane uses compact settings geometry and defers sharing to Save", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1180, height: 850 });
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/?fixture=cloud&state=connected");
  await expect(
    page.getByRole("radio", { name: "Control Plane conversations only" }),
  ).toBeChecked();
  const geometry = await page.locator(".cloud-settings").evaluate((body) => {
    const group = body.querySelector('[role="radiogroup"]')!;
    return {
      gutter: Number.parseFloat(getComputedStyle(body).paddingLeft),
      groupWidth: group.getBoundingClientRect().width,
      rows: [...group.querySelectorAll("button")].map((button) => ({
        width: button.getBoundingClientRect().width,
        height: button.getBoundingClientRect().height,
      })),
    };
  });
  expect(geometry.gutter).toBeGreaterThanOrEqual(20);
  for (const row of geometry.rows) {
    expect(row.height).toBeLessThanOrEqual(40);
    expect(row.width).toBeLessThan(geometry.groupWidth);
  }
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (command: string, parameters: unknown) => Promise<unknown>;
      };
      sharingCalls: unknown[];
    };
    host.sharingCalls = [];
    const invoke = host.__TAURI_INTERNALS__.invoke;
    host.__TAURI_INTERNALS__.invoke = (command, parameters) => {
      if (command === "cloud_set_workspace_sharing")
        host.sharingCalls.push(parameters);
      return invoke(command, parameters);
    };
  });
  const first = page.getByRole("radio", {
    name: "Control Plane conversations only",
  });
  await first.focus();
  await page.keyboard.press("End");
  await expect(
    page.getByRole("radio", { name: "Share history and allow continuation" }),
  ).toBeFocused();
  await expect(
    page.getByRole("radio", { name: "Share history and allow continuation" }),
  ).toBeChecked();
  expect(
    await page.evaluate(
      () => (window as unknown as { sharingCalls: unknown[] }).sharingCalls,
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Save conversation sharing" }).click();
  expect(
    await page.evaluate(
      () => (window as unknown as { sharingCalls: unknown[] }).sharingCalls,
    ),
  ).toEqual([
    { targetId: "preview-workspace", enabled: true, allowContinuation: true },
  ]);
  const result = await new AxeBuilder({ page })
    .include(".managed-settings-shell")
    .analyze();
  expect(result.violations).toEqual([]);
  await page.screenshot({
    path: "output/playwright/control-plane-settings-light.png",
  });
});

test("compact Large settings keep enrollment and global connection editing inside shared gutters", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 850 });
  await page.goto("/?fixture=cloud&state=new");
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
    document.documentElement.dataset.palette = "neutral";
    document.documentElement.dataset.textSize = "large";
  });
  await expect(
    page.getByLabel("Enrollment URL", { exact: true }),
  ).toBeVisible();
  expect(
    await page
      .locator(".cloud-settings")
      .evaluate((body) => body.scrollWidth <= body.clientWidth),
  ).toBe(true);
  const input = await page
    .getByLabel("Enrollment URL", { exact: true })
    .boundingBox();
  const content = await page.locator(".settings-main").boundingBox();
  expect(input!.x).toBeGreaterThan(content!.x);
  expect(input!.x + input!.width).toBeLessThanOrEqual(
    content!.x + content!.width,
  );
  await page.screenshot({
    path: "output/playwright/control-plane-enrollment-compact-large.png",
  });
  await page.goto("/?fixture=cloud&state=connected");
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
    document.documentElement.dataset.palette = "neutral";
    document.documentElement.dataset.textSize = "large";
  });
  await expect(
    page.getByRole("radio", { name: "Control Plane conversations only" }),
  ).toBeChecked();
  expect(
    await page
      .locator(".cloud-settings")
      .evaluate((body) => body.scrollWidth <= body.clientWidth),
  ).toBe(true);
  await page.screenshot({
    path: "output/playwright/control-plane-sharing-compact-large.png",
  });
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page
    .getByRole("button", { name: "Add connection", exact: true })
    .click();
  await page
    .getByLabel("Connection name", { exact: true })
    .fill("Preview team");
  await page
    .getByLabel("Web endpoint", { exact: true })
    .fill("https://control-plane.example.com");
  await page
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Edit Preview team", exact: true }),
  ).toBeVisible();
  expect(
    await page
      .locator(".control-plane-settings")
      .evaluate((body) => body.scrollWidth <= body.clientWidth),
  ).toBe(true);
  await expect(
    page.getByRole("heading", {
      name: "Enrolled workspace runtimes",
      exact: true,
    }),
  ).toBeVisible();
  await page.screenshot({
    path: "output/playwright/control-plane-settings-compact-large.png",
  });
});

test("Connections reports enrolled states, treats bookmarks separately, and manages the exact workspace without activation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
  ).toBeVisible();
  await page.evaluate(async () => {
    const modulePath = "/src/components/ManagedSettingsPane.tsx";
    const { buildManagedSettingsFixture } = await import(modulePath);
    const snapshot = buildManagedSettingsFixture({
      selectedSpaceId: "fixture-managed-local",
      spaces: [
        {
          spaceId: "fixture-managed-local",
          displayName: "Colossus",
          displayPath: "~/tools/Colossus",
          archived: false,
        },
        {
          spaceId: "fixture-research",
          displayName: "Research Lab",
          displayPath: "~/tools/research-lab",
          archived: false,
        },
      ],
      managedModelConfiguration: { providers: [], models: [], roles: {} },
      accessProfile: "minimal",
      executionBoundary: "offline_isolated",
    });
    const catalog = {
      revision: 1,
      profiles: [
        {
          id: "sample",
          label: "Saved team endpoint",
          endpoint: "https://bookmark.example.test",
        },
      ],
      defaultProfile: "sample",
      connections: [
        {
          targetId: "fixture-managed-local",
          status: "connected",
          projectId: "sample-project",
          endpoint: "https://control-plane.example.test",
          sharedSessions: false,
        },
        {
          targetId: "fixture-research",
          status: "disconnected",
          projectId: "sample-project",
          endpoint: "https://control-plane.example.test",
          sharedSessions: false,
        },
      ],
    };
    const host = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      controlPlaneFixture: {
        fail: boolean;
        calls: { command: string; parameters: unknown }[];
      };
    };
    host.controlPlaneFixture = { fail: false, calls: [] };
    host.__TAURI_INTERNALS__ = {
      invoke: async (
        command: string,
        parameters: { targetId?: string } = {},
      ) => {
        host.controlPlaneFixture.calls.push({ command, parameters });
        if (command === "control_plane_profiles") {
          if (host.controlPlaneFixture.fail)
            throw new Error("Synthetic status unavailable");
          return catalog;
        }
        if (command === "get_managed_configuration") return snapshot;
        if (command === "list_setup_packages") return [];
        if (command === "cloud_status")
          return {
            ...catalog.connections[1],
            targetId: parameters.targetId,
            nodeId: "sample-node",
            hostId: "sample-host",
            sharingSupported: true,
            sharedContinuation: false,
          };
        throw new Error("Unsupported read-only fixture command");
      },
    };
  });
  await page.getByRole("button", { name: "Connections", exact: true }).click();
  const plane = page.getByRole("region", {
    name: "Control Plane connections",
    exact: true,
  });
  await expect(plane.getByText("Connected", { exact: true })).toBeVisible();
  await expect(plane.getByText("Disconnected", { exact: true })).toBeVisible();
  await expect(
    plane.getByText("Default bookmark", { exact: true }),
  ).toBeVisible();
  await expect(
    plane
      .locator(".control-plane-bookmarks")
      .getByText("Connected", { exact: true }),
  ).toHaveCount(0);
  await page.screenshot({
    path: "output/playwright/control-plane-connections.png",
  });
  await page.evaluate(() => {
    (
      window as unknown as { controlPlaneFixture: { fail: boolean } }
    ).controlPlaneFixture.fail = true;
  });
  await plane
    .getByRole("button", { name: "Refresh status", exact: true })
    .click();
  await expect(plane.getByText("Unknown", { exact: true })).toHaveCount(2);
  await expect(plane.getByText("Connected", { exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    (
      window as unknown as { controlPlaneFixture: { fail: boolean } }
    ).controlPlaneFixture.fail = false;
  });
  await plane
    .getByRole("button", { name: "Refresh status", exact: true })
    .click();
  await expect(plane.getByText("Connected", { exact: true })).toBeVisible();
  await plane
    .getByRole("button", {
      name: "Manage Control Plane for Research Lab",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("radio", {
      name: "Control Plane conversations only",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.locator(".space-settings-context .app-select-trigger"),
  ).toContainText("Research Lab");
  const calls = await page.evaluate(
    () =>
      (
        window as unknown as {
          controlPlaneFixture: {
            calls: { command: string; parameters: { targetId?: string } }[];
          };
        }
      ).controlPlaneFixture.calls,
  );
  expect(
    calls.some(
      (call) =>
        call.command === "cloud_status" &&
        call.parameters.targetId === "fixture-research",
    ),
  ).toBe(true);
  expect(
    calls.some((call) =>
      [
        "select_target",
        "select_space",
        "cloud_connect",
        "cloud_enroll",
        "cloud_set_workspace_sharing",
      ].includes(call.command),
    ),
  ).toBe(false);
});
