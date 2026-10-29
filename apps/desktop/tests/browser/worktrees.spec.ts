import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { installDesktopExtrasFixture } from "./support/desktop-extras";

async function component(page: Page, external = false) {
  await page.route("**/worktree-component-fixture*", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<!doctype html><html lang="en"><head><title>Owned worktree fixture</title><script type="module">import RefreshRuntime from "/@react-refresh"; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => (type) => type; window.__vite_plugin_react_preamble_installed__ = true;</script></head><body><div id="root"></div><script type="module" src="/src/dev/worktree-settings-fixture.tsx"></script></body></html>',
    }),
  );
  await page.goto(
    `/worktree-component-fixture${external ? "?external=1" : ""}`,
  );
  await expect(
    page.getByRole("heading", { name: "Worktrees", exact: true }),
  ).toBeVisible();
}
async function allow(page: Page) {
  await page.getByRole("button", { name: "Allow worktrees…" }).click();
  await expect(
    page.getByRole("textbox", { name: "Name", exact: true }),
  ).toBeVisible();
}
async function create(page: Page, name: string) {
  await page.getByRole("textbox", { name: "Name", exact: true }).fill(name);
  await page.getByRole("button", { name: "Create worktree…" }).click();
  await expect(page.getByText(name, { exact: true })).toBeVisible();
}

test("actual Settings mount sends the selected workspace identity rather than its space ID", async ({
  page,
}) => {
  await installDesktopExtrasFixture(page);
  await page.addInitScript(() => {
    const fixture = { requests: [] as unknown[] };
    Object.assign(window, { worktreeMountFixture: fixture });
    let value = (
      window as unknown as { __TAURI_INTERNALS__?: Record<string, unknown> }
    ).__TAURI_INTERNALS__;
    const wrap = (next: Record<string, unknown> | undefined) => {
      if (!next) return next;
      const original = next.invoke as (
        command: string,
        args?: Record<string, unknown>,
      ) => Promise<unknown>;
      return {
        ...next,
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          if (command === "get_worktree_preferences")
            return {
              directory: "C:\\OwnedFixture\\ColossusWorktrees",
              settings: { branchPrefix: "colossus/", openAfterCreate: true },
            };
          if (command === "get_workspace_worktrees") {
            fixture.requests.push(args.request);
            if (
              (args.request as { workspaceId: string }).workspaceId !==
              "fixture-workspace"
            )
              throw {
                code: "fixture_wrong_identity",
                message:
                  "The mount sent a space ID instead of workspace identity.",
                retryable: false,
                outcomeUnknown: false,
                violations: [],
              };
            return { state: "permission_required", catalog: null };
          }
          return original(command, args);
        },
      };
    };
    value = wrap(value);
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      get: () => value,
      set: (next: Record<string, unknown>) => {
        value = wrap(next);
      },
    });
  });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Worktrees", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Allow worktrees…" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { worktreeMountFixture: { requests: unknown[] } })
          .worktreeMountFixture.requests,
    ),
  ).toEqual([{ workspaceId: "fixture-workspace", approveRepository: false }]);
});

test("create, archive, restore and clean removal preserve dirty checkouts and use defaults", async ({
  page,
}) => {
  await component(page);
  await allow(page);
  await expect(
    page.getByText("C:\\OwnedFixture\\ColossusWorktrees", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("textbox", { name: "New branch prefix" })
    .fill("feature/");
  await page
    .getByRole("checkbox", { name: "Open in Desktop after creation" })
    .uncheck();
  await page.getByRole("button", { name: "Save defaults" }).click();
  await create(page, "review-fix");
  await expect(
    page.getByText("feature/review-fix · active", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Worktree updated");
  const row = page.locator(".worktree-list li").first();
  await row.getByRole("button", { name: "Archive…" }).click();
  await expect(
    row.getByRole("button", { name: "Restore and open…" }),
  ).toBeEnabled();
  await page.evaluate(() => {
    const fixture = (
      window as unknown as {
        worktreeFixture: { dirty(id: string, value: boolean): void };
      }
    ).worktreeFixture;
    fixture.dirty("1".padStart(32, "0"), true);
  });
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    row.getByRole("button", { name: "Remove checkout…" }),
  ).toBeDisabled();
  await expect(row).toContainText("has local files or changes");
  await row.getByRole("button", { name: "Restore and open…" }).click();
  await expect(page.getByLabel("Workspaces opened")).toHaveText("1");
  await row.getByRole("button", { name: "Archive…" }).click();
  await page.evaluate(() =>
    (
      window as unknown as {
        worktreeFixture: { dirty(id: string, value: boolean): void };
      }
    ).worktreeFixture.dirty("1".padStart(32, "0"), false),
  );
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await row.getByRole("button", { name: "Remove checkout…" }).click();
  await expect(row).toContainText("removed");
  await expect(page.getByRole("status")).toContainText(
    "Its branch was retained",
  );
  await page.getByRole("button", { name: "Choose folder…" }).click();
  await expect(
    page.getByText("C:\\AnotherOwnedFixture\\ColossusWorktrees", {
      exact: true,
    }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Use default", exact: true }).click();
  await expect(
    page.getByText("C:\\OwnedFixture\\ColossusWorktrees", { exact: true }),
  ).toBeVisible();
});

test("uncertain mutation blocks another change until refresh without repeating creation", async ({
  page,
}) => {
  await component(page);
  await allow(page);
  await page.evaluate(() => {
    (
      window as unknown as { worktreeFixture: { failNext: boolean } }
    ).worktreeFixture.failNext = true;
  });
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("uncertain");
  await page.getByRole("button", { name: "Create worktree…" }).click();
  await expect(page.getByRole("alert")).toContainText("outcome is unknown");
  await expect(
    page.getByRole("button", { name: "Create worktree…" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(page.getByText("uncertain", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Create worktree…" }),
  ).toBeEnabled();
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as {
            worktreeFixture: { calls: { kind: string }[] };
          }
        ).worktreeFixture.calls.filter((call) => call.kind === "create").length,
    ),
  ).toBe(1);
});

test("an external host selection does not request repository access", async ({
  page,
}) => {
  await component(page, true);
  await expect(
    page.getByText(
      "Select a local workspace with file access to manage worktrees.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Refresh", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Create worktree…" }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as {
            worktreeFixture: { calls: { kind: string }[] };
          }
        ).worktreeFixture.calls.filter((call) => call.kind === "list").length,
    ),
  ).toBe(0);
});

for (const [theme, palette] of [
  ["light", "colossus"],
  ["dark", "colossus"],
  ["dark", "neutral"],
  ["dark", "black"],
  ["dark", "hacker"],
] as const) {
  test(`worktree settings remain usable with Large text in ${theme} ${palette}`, async ({
    page,
  }, testInfo) => {
    await page.addInitScript(
      ({ theme, palette }) => {
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({
            colorTheme: theme,
            darkPalette: palette,
            textSize: "large",
          }),
        );
      },
      { theme, palette },
    );
    await component(page);
    await allow(page);
    const name = page.getByRole("textbox", { name: "Name", exact: true });
    await name.focus();
    await page.keyboard.type("keyboard-feature");
    await page.keyboard.press("Enter");
    await expect(
      page.getByText("keyboard-feature", { exact: true }),
    ).toBeVisible();
    await expect(page.getByLabel("Workspaces opened")).toHaveText("1");
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    const surface = page.getByRole("region", {
      name: "Worktrees",
      exact: true,
    });
    await surface.screenshot({
      path: testInfo.outputPath("worktrees-wide.png"),
    });
    await page.setViewportSize({ width: 700, height: 990 });
    await surface.screenshot({
      path: testInfo.outputPath("worktrees-compact.png"),
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      (await new AxeBuilder({ page }).include(".worktree-settings").analyze())
        .violations,
    ).toEqual([]);
  });
}

test("maximum length branch and inventory names wrap in compact Large settings", async ({
  page,
}) => {
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.desktop.appearance.v1",
      JSON.stringify({
        colorTheme: "dark",
        darkPalette: "neutral",
        textSize: "large",
      }),
    ),
  );
  await page.setViewportSize({ width: 700, height: 990 });
  await component(page);
  await allow(page);
  await page
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("x".repeat(64));
  await page
    .getByRole("textbox", { name: "Branch (optional)", exact: true })
    .fill("feature/" + "y".repeat(190));
  await page.getByRole("button", { name: "Create worktree…" }).click();
  await expect(page.locator(".worktree-list li")).toHaveCount(1);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
});
