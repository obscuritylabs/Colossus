import { expect, test, type Page } from "@playwright/test";
import { browserFixture } from "./fixture";
import {
  appearanceCases,
  openAppSidebar,
  selectAppearance,
} from "./appearance";
import type { HostedActivity, HostedTarget } from "../../src/hosted-types";
import type { Task } from "../../src/api";
import type { Me } from "../../src/control-api";

const base = "/api/projects/browser-project/hosted";
async function hostedFixture(
  page: Page,
  options: {
    disabled?: boolean;
    waiting?: boolean;
    permissions?: Me["memberships"][number]["permissions"];
  } = {},
) {
  const fixture = await browserFixture(page, {
    managedChat: true,
    ...(options.permissions ? { permissions: options.permissions } : {}),
  });
  const target: HostedTarget = {
    node: {
      project_id: "browser-project",
      node_id: fixture.nodeId,
      instance_id: "hosted-chat-browser",
      label: "Colossus Hosted",
      owner_user_id: "browser-user",
      execution_kind: "hosted",
      certificate_sha256: "",
      roles: ["primary"],
      revoked: false,
      revision: 1,
    },
    presence: {
      ready: true,
      connection_id: "hosted-config-1",
      capabilities: [],
    },
    execution: {
      kind: "hosted",
      state: "on_demand",
      environment: null,
      managed_files: true,
      filesystem: false,
      process: false,
    },
  };
  const activity: HostedActivity = {
    queued: 0,
    running: 0,
    waiting: options.waiting ? 1 : 0,
    limited: false,
  };
  if (options.waiting) target.execution.state = "warm";
  const task = (
    id: string,
    status: "completed" | "failed" | "waiting",
  ): Task => ({
    project_id: "browser-project",
    node_id: fixture.nodeId,
    task_id: id,
    thread_id: "browser-thread",
    subject: "browser-user",
    run_id: id,
    request: {
      input: [
        {
          text:
            status === "failed"
              ? "Failed evidence summary"
              : status === "waiting"
                ? "Review pending response"
                : "Completed evidence summary",
        },
      ],
      mode: "execute",
      role: "primary",
    },
    created_at: "2026-10-20T10:00:00Z",
    revision: 1,
    last_sequence: 1,
    snapshot: {
      run: {
        run_id: id,
        session_id: "browser-session",
        status,
        role: "primary",
        mode: "execute",
        created_at: "2026-10-20T10:00:00Z",
        updated_at: "2026-10-20T10:00:00Z",
        last_sequence: 1,
      },
      pending_interactions: [],
    },
  });
  const mutations: string[] = [],
    reads: string[] = [];
  let rejectRelease = false,
    failInventory = false;
  await page.route(
    "**/api/projects/browser-project/hosted{,/**,?*}",
    async (route) => {
      const request = route.request(),
        url = new URL(request.url());
      if (request.method() === "GET") {
        reads.push(url.pathname + url.search);
        if (url.pathname === base)
          return route.fulfill({
            status: failInventory ? 503 : 200,
            json: failInventory
              ? { error: "storage" }
              : {
                  enabled: !options.disabled,
                  target: options.disabled ? null : target,
                  activity,
                },
          });
        if (url.pathname === `${base}/tasks`)
          return route.fulfill({
            json: {
              tasks:
                url.searchParams.get("active") === "true"
                  ? activity.waiting
                    ? [task("waiting-run", "waiting")]
                    : []
                  : [
                      url.searchParams.has("after")
                        ? task("older-run", "failed")
                        : task("recent-run", "completed"),
                    ],
              next_cursor:
                url.searchParams.get("active") === "true" ||
                url.searchParams.has("after")
                  ? null
                  : "synthetic-next-page",
            },
          });
      } else {
        expect(request.headers()["x-colossus-csrf"]).toBe("1");
        mutations.push(url.pathname);
        if (url.pathname.endsWith("/release") && rejectRelease)
          return route.fulfill({ status: 409, json: { error: "conflict" } });
        target.execution.state = url.pathname.endsWith("/load")
          ? "warm"
          : "on_demand";
        return route.fulfill({
          json: url.pathname.endsWith("/load")
            ? { loaded: true }
            : { released: true },
        });
      }
      throw new Error(
        `Unhandled Hosted request: ${request.method()} ${url.pathname}`,
      );
    },
  );
  return {
    ...fixture,
    mutations,
    reads,
    detail: `/projects/browser-project/hosted/${fixture.nodeId}`,
    rejectRelease: () => {
      rejectRelease = true;
    },
    failInventory: () => {
      failInventory = true;
    },
  };
}

for (const appearance of appearanceCases) {
  test(`Hosted inventory, lifecycle and history in ${appearance.label}`, async ({
    page,
  }) => {
    const fixture = await hostedFixture(page);
    await selectAppearance(page, appearance);
    await openAppSidebar(page);
    const hostedLink = page.getByRole("link", { name: "Hosted", exact: true });
    await hostedLink.focus();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/hosted\?project=browser-project$/u);
    await expect(
      page.getByRole("table", { name: "Hosted agents", exact: true }),
    ).toBeVisible();
    expect(fixture.mutations).toEqual([]);
    await page
      .getByRole("link", { name: "Colossus Hosted", exact: true })
      .click();
    await expect(page).toHaveURL(fixture.detail);
    await expect(
      page.getByRole("heading", { name: "Colossus Hosted", exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: `output/playwright/hosted-${appearance.id}-overview-narrow.png`,
      fullPage: true,
    });
    await expect(
      page.getByRole("region", { name: "Execution access" }),
    ).toContainText("Filesystem accessUnavailable");
    const load = page.getByRole("button", {
      name: "Load runtime",
      exact: true,
    });
    await load.focus();
    await page.keyboard.press("Enter");
    await expect(
      page.getByRole("status").filter({ hasText: "Runtime loaded" }),
    ).toBeVisible();
    const release = page.getByRole("button", {
      name: "Release idle runtime",
      exact: true,
    });
    await expect(release).toBeEnabled();
    await release.click();
    await expect(
      page.getByRole("status").filter({ hasText: "Idle runtime released" }),
    ).toBeVisible();
    await expect(load).toBeEnabled();
    await page.getByRole("button", { name: "All runs", exact: true }).click();
    await expect(
      page.getByRole("link", { name: "Completed evidence summary" }),
    ).toHaveAttribute("href", /\/tasks\/recent-run$/u);
    await page.getByRole("button", { name: "More runs", exact: true }).click();
    await expect(
      page.getByRole("link", { name: "Failed evidence summary" }),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Previous runs", exact: true })
      .click();
    await expect(
      page.getByRole("link", { name: "Completed evidence summary" }),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/hosted-${appearance.id}-narrow.png`,
      fullPage: true,
    });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page
      .getByRole("heading", { name: "Colossus Hosted", exact: true })
      .scrollIntoViewIfNeeded();
    await page.screenshot({
      path: `output/playwright/hosted-${appearance.id}-desktop.png`,
      fullPage: true,
    });
    await page
      .getByRole("table", { name: "Hosted run history" })
      .getByRole("link", { name: "Open chat", exact: true })
      .click();
    await expect(page).toHaveURL(/\/chat\/browser-thread$/u);
    await expect(
      page.getByText("Retained personal chat question", { exact: true }),
    ).toBeVisible();
    await page.goBack();
    await page.reload();
    await expect(
      page.getByRole("heading", { name: "Colossus Hosted", exact: true }),
    ).toBeVisible();
    expect(fixture.mutations).toEqual([
      `${base}/runtime/load`,
      `${base}/runtime/release`,
    ]);
    fixture.verify();
  });
}

test("Hosted navigation preserves an unfinished chat draft", async ({
  page,
}) => {
  const fixture = await hostedFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/projects/browser-project/chat/browser-thread");
  const composer = page.getByRole("textbox", {
    name: "Message the agent",
    exact: true,
  });
  await composer.fill("Keep this draft while inspecting Hosted execution");
  await page.getByRole("link", { name: "Hosted", exact: true }).click();
  await page
    .getByRole("link", { name: "Colossus Hosted", exact: true })
    .click();
  await page
    .getByRole("region", { name: "Runtime lifecycle" })
    .getByRole("link", { name: "Open chat", exact: true })
    .click();
  await expect(composer).toHaveValue(
    "Keep this draft while inspecting Hosted execution",
  );
  expect(fixture.mutations).toEqual([]);
  fixture.verify();
});

test("Hosted inventory respects compact text, Black palette and forced colors", async ({
  page,
}) => {
  const fixture = await hostedFixture(page);
  await page.goto("/settings?project=browser-project");
  await page.locator("#appearance-text-size").click();
  await page.getByRole("option", { name: "Compact", exact: true }).click();
  await page.locator("#appearance-color-theme").click();
  await page.getByRole("option", { name: "Dark", exact: true }).click();
  await page.locator("#appearance-dark-palette").click();
  await page
    .getByRole("option", { name: "Black (shadcn)", exact: true })
    .click();
  await page.goto("/hosted?project=browser-project");
  await expect(page.locator("html")).toHaveAttribute(
    "data-text-size",
    "compact",
  );
  await expect(page.locator("html")).toHaveAttribute("data-palette", "black");
  await expect(
    page.getByRole("link", { name: "Colossus Hosted", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: "output/playwright/hosted-black-compact.png",
    fullPage: true,
  });
  await page.emulateMedia({ forcedColors: "active" });
  await page
    .getByRole("link", { name: "Colossus Hosted", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Colossus Hosted", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "output/playwright/hosted-forced-colors.png",
    fullPage: true,
  });
  fixture.verify();
});

test("waiting work pins runtime and exposes its run", async ({ page }) => {
  const fixture = await hostedFixture(page, { waiting: true });
  await page.goto(fixture.detail);
  await expect(
    page.getByRole("link", { name: "Review pending response" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Release idle runtime" }),
  ).toBeDisabled();
  await expect(
    page.getByText("Active or waiting work keeps the runtime loaded.", {
      exact: false,
    }),
  ).toBeVisible();
  expect(fixture.mutations).toEqual([]);
  fixture.verify();
});

test("a release race is reported without retrying or claiming success", async ({
  page,
}) => {
  const fixture = await hostedFixture(page);
  await page.goto(fixture.detail);
  await page.getByRole("button", { name: "Load runtime", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Release idle runtime" }),
  ).toBeEnabled();
  fixture.rejectRelease();
  await page.getByRole("button", { name: "Release idle runtime" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "busy or work needs reconciliation",
  );
  expect(
    fixture.mutations.filter((path) => path.endsWith("/release")),
  ).toHaveLength(1);
  await expect(
    page.getByText("Idle runtime released", { exact: false }),
  ).toHaveCount(0);
  fixture.verify([
    "Failed to load resource: the server responded with a status of 409 (Conflict)",
  ]);
});

test("readers can inspect history without lifecycle authority", async ({
  page,
}) => {
  const fixture = await hostedFixture(page, { permissions: ["read"] });
  await page.goto(fixture.detail);
  await expect(
    page.getByRole("heading", { name: "Colossus Hosted", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Load runtime", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Release idle runtime" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Project settings" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "All runs", exact: true }).click();
  await expect(
    page.getByRole("link", { name: "Completed evidence summary" }),
  ).toBeVisible();
  expect(fixture.mutations).toEqual([]);
  fixture.verify();
});

test("a foreign Hosted permalink cannot select another owner's target", async ({
  page,
}) => {
  const fixture = await hostedFixture(page);
  await page.goto("/projects/browser-project/hosted/foreign-target");
  await expect(page.getByRole("alert")).toContainText(
    "unavailable in the selected project",
  );
  await expect(
    page.getByRole("button", { name: "Load runtime", exact: true }),
  ).toHaveCount(0);
  expect(fixture.reads.some((path) => path.includes("foreign-target"))).toBe(
    false,
  );
  expect(fixture.mutations).toEqual([]);
  fixture.verify();
});

test("disabled Hosted execution has an explicit empty state", async ({
  page,
}) => {
  const fixture = await hostedFixture(page, { disabled: true });
  await page.goto("/hosted?project=browser-project");
  await expect(
    page.getByRole("heading", { name: "Hosted agents are not enabled" }),
  ).toBeVisible();
  expect(fixture.mutations).toEqual([]);
  fixture.verify();
});

test("inventory read failures hide stale lifecycle controls", async ({
  page,
}) => {
  const fixture = await hostedFixture(page);
  await page.goto(fixture.detail);
  await expect(
    page.getByRole("button", { name: "Load runtime", exact: true }),
  ).toBeVisible();
  fixture.failInventory();
  await page.getByRole("button", { name: "Refresh hosted agents" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "temporarily unavailable",
  );
  await expect(
    page.getByRole("button", { name: "Load runtime", exact: true }),
  ).toHaveCount(0);
  expect(fixture.mutations).toEqual([]);
  fixture.verify([
    "Failed to load resource: the server responded with a status of 503 (Service Unavailable)",
  ]);
});
