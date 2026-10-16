import { expect, test } from "@playwright/test";
import type { Operation } from "../../src/client";

test("server selection, local sign-in, and browser SSO remain distinct", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/?fixture=login");
  await expect(
    page.getByRole("heading", { name: "Connect to Colossus" }),
  ).toBeVisible();
  await page.getByLabel("Server address").fill("https://colossus.example/api");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("without a path");
  await page.getByLabel("Server address").fill("https://colossus.example");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Sign in to Colossus" }),
  ).toBeVisible();
  await expect(
    page.getByText("Browser sign-in stays in Control Plane", { exact: false }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/mobile-login.png",
    fullPage: true,
  });
  await page.getByLabel("Username", { exact: true }).fill("preview");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Recent work" }),
  ).toBeVisible();
  await expect(page.locator(".mobile-row").nth(1)).toBeInViewport({ ratio: 1 });
  const storage = await page.evaluate(() =>
    JSON.stringify({
      local: { ...localStorage },
      session: { ...sessionStorage },
    }),
  );
  expect(storage).not.toContain("fixture-password");
  expect(storage).not.toContain("session=");
  expect(errors).toEqual([]);
});

test("work, fleet, inbox and accessible appearance preferences share one shell", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await expect(
    page.getByText("Development preview · Sample data", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Recent work" }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/mobile-work.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Fleet", exact: true }).click();
  await expect(page.getByText("Windows dev", { exact: true })).toBeVisible();
  await expect(page.getByText("Linux build", { exact: true })).toBeVisible();
  await expect(page.getByText("Mac studio", { exact: true })).toBeVisible();
  await page.screenshot({
    path: "test-results/mobile-fleet.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Review in Control Plane" }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/mobile-inbox.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("combobox", { name: "Color theme", exact: true })
    .click();
  await page.getByRole("option", { name: "Light", exact: true }).click();
  await page.getByRole("combobox", { name: "Text size", exact: true }).click();
  await page.getByRole("option", { name: "Large", exact: true }).click();
  await page.setViewportSize({ width: 320, height: 844 });
  await expect(page.locator("html")).toHaveAttribute("data-text-size", "large");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/mobile-settings-large-light.png",
    fullPage: true,
  });
});

test("a new conversation submits to the selected workspace and can continue", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Review the reconnect behavior");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(
    page.getByRole("heading", {
      name: "Review the reconnect behavior",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByText("Review the reconnect behavior", { exact: true }).last(),
  ).toBeVisible();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Check cancellation too");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(
    page.getByText("Check cancellation too", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Send", exact: true }),
  ).toBeInViewport();
  await page.screenshot({
    path: "test-results/mobile-conversation.png",
    fullPage: true,
  });
});

test("an uncertain submission reconciles the original operation after fleet changes", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const attempts: Operation[] = [];
    let accepted: unknown;
    Object.defineProperty(globalThis, "isTauri", { value: true });
    Object.defineProperty(globalThis, "__mobileAttempts", { value: attempts });
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        async invoke(
          command: string,
          args: {
            operation?: Operation;
            origin?: string;
            username?: string;
            password?: string;
          },
        ) {
          const fixturePath = "/src/preview.ts";
          const { previewClient } = await import(fixturePath);
          if (command === "mobile_connect")
            return previewClient.connect(args.origin);
          if (command === "mobile_login")
            return previewClient.login(args.username, args.password);
          if (command !== "mobile_request")
            throw new Error("Unexpected native command");
          const operation = args.operation!;
          if (operation.kind === "create_thread") {
            attempts.push(structuredClone(operation));
            if (attempts.length === 1) {
              accepted = await previewClient.request(operation);
              throw {
                code: "outcome_unknown",
                message: "The result could not be confirmed.",
              };
            }
            if (JSON.stringify(operation) !== JSON.stringify(attempts[0]))
              throw {
                code: "conflict",
                message: "Reconciliation changed the original request.",
              };
            return accepted;
          }
          const result = await previewClient.request(operation);
          if (operation.kind === "fleet" && attempts.length)
            result.nodes.forEach((item: { node: { roles: string[] } }) => {
              item.node.roles = ["reviewer"];
            });
          return result;
        },
      },
    });
  });
  await page.goto("/");
  await page.getByLabel("Server address").fill("https://colossus.example");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByLabel("Username", { exact: true }).fill("preview");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Keep the original allocation");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("could not be confirmed");
  await expect(
    page.getByRole("button", { name: "Settings", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("textbox", { name: "Message Colossus" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Task options · Execute" }).click();
  await expect(
    page.getByRole("radio", { name: "reviewer", exact: true }),
  ).toBeDisabled({ timeout: 10000 });
  await page.getByRole("button", { name: "Done", exact: true }).click();
  expect(
    await page.evaluate(
      () => Reflect.get(globalThis, "__mobileAttempts").length,
    ),
  ).toBe(1);
  await page
    .getByRole("button", {
      name: "Reconcile the original submission",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Keep the original allocation",
      exact: true,
    }),
  ).toBeVisible();
  const operations = await page.evaluate(() =>
    Reflect.get(globalThis, "__mobileAttempts"),
  );
  expect(operations).toHaveLength(2);
  expect(operations[1]).toEqual(operations[0]);
});

test("password visibility, failed login, and changing servers preserve credential custody", async ({
  page,
}) => {
  await page.goto("/?fixture=login");
  await page.getByLabel("Server address").fill("https://colossus.example");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByLabel("Username", { exact: true }).fill("invalid");
  await page.getByLabel("Password", { exact: true }).fill("fixture-password");
  await page
    .getByRole("button", { name: "Show password", exact: true })
    .click();
  await expect(page.getByLabel("Password", { exact: true })).toHaveAttribute(
    "type",
    "text",
  );
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Sign-in failed");
  await expect(page.getByLabel("Password", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("Password", { exact: true })).toHaveAttribute(
    "type",
    "password",
  );
  await expect(page.getByLabel("Password", { exact: true })).toBeFocused();
  await page
    .getByRole("button", { name: "Change server", exact: true })
    .click();
  await expect(page.getByLabel("Server address")).toBeFocused();
  expect(
    await page.evaluate(() =>
      JSON.stringify({ ...localStorage, ...sessionStorage }),
    ),
  ).not.toContain("fixture-password");
});

test("task inventory opens released output, event history, and its conversation", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Recent tasks" }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/mobile-tasks.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: /Prepare.*release notes/ }).click();
  await expect(
    page.getByRole("heading", { name: "Run output", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Release notes ready", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Cancel task", exact: true }),
  ).toHaveCount(0);
  await page.getByText("Event activity · 4 events", { exact: true }).click();
  await expect(page.locator(".mobile-task-events li")).toHaveCount(4);
  await page.screenshot({
    path: "test-results/mobile-task-completed.png",
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Open conversation", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Prepare release notes", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Release notes ready", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/mobile-messages.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page
    .getByRole("button", { name: /Investigate.*connection timeout/ })
    .click();
  await page.getByRole("button", { name: "Cancel task", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Cancellation requested" }),
  ).toContainText("Cancellation requested");
  await expect(page.getByText("Cancelled", { exact: true })).toBeVisible();
});

test("retained task output catches up across pages and keeps one canonical result", async ({
  page,
}) => {
  await page.route("**/src/preview.ts*", async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    await route.fulfill({
      response,
      body:
        source +
        `
const originalRequest = previewClient.request.bind(previewClient);
previewClient.request = async (operation) => {
  if (operation.kind !== "task_updates" || operation.task !== "task-2") return originalRequest(operation);
  const history = Array.from({ length: 40 }, (_, index) => ({ run_id: "run-2", sequence: index + 1, created_at: new Date().toISOString(), update: index === 39 ? { result: { output: "The complete retained result." } } : { output_delta: "Partial output. " } }));
  const remaining = history.filter((event) => event.sequence > operation.after);
  return { updates: remaining.slice(0, 32), next_after: remaining.length > 32 ? remaining[31].sequence : null };
};
`,
    });
  });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByRole("button", { name: /Prepare.*release notes/ }).click();
  await expect(
    page.getByText("The complete retained result.", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Partial output.", { exact: false })).toHaveCount(
    0,
  );
  await page.getByRole("button", { name: "Refresh task", exact: true }).click();
  await page.getByText("Event activity · 40 events", { exact: true }).click();
  await expect(page.locator(".mobile-task-events li")).toHaveCount(40);
});

test("sign-in keeps its hierarchy at phone width with Large text across palettes", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 844 });
  await page.goto("/?fixture=login");
  await expect(
    page.getByRole("heading", { name: "Connect to Colossus" }),
  ).toBeVisible();
  for (const [theme, palette] of [
    ["dark", "black"],
    ["dark", "neutral"],
    ["dark", "colossus"],
    ["dark", "hacker"],
    ["light", "black"],
  ]) {
    await page.evaluate(
      ({ theme, palette }) => {
        localStorage.setItem("colossus.mobile.theme", theme!);
        localStorage.setItem("colossus.mobile.palette", palette!);
        localStorage.setItem("colossus.mobile.text-size", "large");
      },
      { theme, palette },
    );
    await page.reload();
    await page.getByLabel("Server address").fill("https://colossus.example");
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    await expect(page.getByLabel("Username", { exact: true })).toBeVisible();
    const serverName = page.locator(".mobile-server-selection strong");
    expect((await serverName.boundingBox())!.height).toBeLessThanOrEqual(
      await serverName.evaluate(
        (element) => parseFloat(getComputedStyle(element).lineHeight) + 1,
      ),
    );
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
  await page.screenshot({
    path: "test-results/mobile-login-large-light.png",
    fullPage: true,
  });
});

test("queued work cannot request cancellation before native run allocation", async ({
  page,
}) => {
  await page.route("**/src/preview.ts*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body:
        (await response.text()) +
        `
tasks[0] = { ...tasks[0], request: { ...tasks[0].request, input: [{ text: "Queued before allocation" }] }, run_id: null, snapshot: null };
activity.set("task-0", []);
`,
    });
  });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByRole("button", { name: /Queued before allocation/ }).click();
  await expect(
    page.getByRole("button", { name: "Cancel task", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText(
      "Cancellation becomes available after the runtime allocates a run.",
      { exact: true },
    ),
  ).toBeVisible();
});
