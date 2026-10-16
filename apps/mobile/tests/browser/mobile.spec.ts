import { expect, test } from "@playwright/test";
import type { Operation } from "../../src/client";

test("server selection, local sign-in, and browser SSO remain distinct", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/?fixture=login");
  await expect(
    page.getByRole("heading", { name: "Your agents. Wherever you are." }),
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
  await expect(
    page.getByRole("combobox", { name: "Agent role", exact: true }),
  ).toContainText("reviewer", { timeout: 10000 });
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
