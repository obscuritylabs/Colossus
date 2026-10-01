import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("command card exposes the full tail by keyboard without approving", async ({
  page,
}) => {
  await page.goto("/?fixture=command-approval");
  const command = page.getByRole("region", {
    name: "Prepared command",
    exact: true,
  });
  await expect(command).toBeVisible();
  await expect(command).toContainText(
    "Check the workspace build before applying the requested fix.",
  );
  await expect(command).toContainText("/work/project");
  await expect(command).toContainText("Credential-bearing text is redacted");
  await expect(command).not.toContainText("COMMAND_TAIL");
  const expand = command.getByRole("button", { name: "Show full command" });
  await expand.focus();
  await page.keyboard.press("Enter");
  const full = command.getByRole("region", {
    name: "Full prepared argument vector, executable first",
  });
  await expect(full).toContainText("COMMAND_TAIL");
  expect(await full.textContent()).toContain("two  spaces");
  await full.focus();
  await page.keyboard.press("End");
  await expect(
    page.getByRole("button", { name: "Review command…", exact: true }),
  ).toBeEnabled();
  await page.setViewportSize({ width: 880, height: 540 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(
    880,
  );
  const scan = await new AxeBuilder({ page }).analyze();
  expect(
    scan.violations.filter((item) =>
      ["critical", "serious"].includes(item.impact ?? ""),
    ),
  ).toEqual([]);
});

for (const decision of ["deny", "allow_once", "always_allow"] as const) {
  test(`native review document forwards only its one-use identity: ${decision}`, async ({
    page,
  }) => {
    await page.addInitScript(() => {
      const calls: unknown[] = [];
      Object.assign(window, {
        reviewCalls: calls,
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, args });
            if (command === "command_review_context")
              return {
                reviewId: "native-review-id",
                target: "Managed Local — isolated workspace",
                canRemember: true,
                commandContext: {
                  justification: "Check the workspace build.",
                  executable: "/bin/sh",
                  arguments: [
                    "-c",
                    `echo '<script>plain text</script>'; # ${"x".repeat(8000)}COMMAND_TAIL`,
                  ],
                  workingDirectory: "/work/project",
                  redacted: false,
                },
              };
            if (command === "finish_command_review") return;
            throw new Error("Unexpected native command");
          },
        },
      });
    });
    await page.goto("/?surface=command-approval");
    await expect(
      page.getByRole("heading", { name: "Review command" }),
    ).toBeVisible();
    await expect(
      page.getByRole("region", {
        name: "Full prepared argument vector, executable first",
      }),
    ).toContainText("COMMAND_TAIL");
    await expect(page.locator(".command-approval-preview")).toHaveCount(0);
    const full = page.getByRole("region", {
      name: "Full prepared argument vector, executable first",
    });
    await full.focus();
    await page.keyboard.press("End");
    await expect
      .poll(() =>
        full.evaluate(
          (element) =>
            element.scrollTop + element.clientHeight >=
            element.scrollHeight - 1,
        ),
      )
      .toBe(true);
    await expect(page.locator("script", { hasText: "plain text" })).toHaveCount(
      0,
    );
    if (decision === "always_allow") {
      await page.setViewportSize({ width: 900, height: 760 });
      await page.screenshot({ path: "output/playwright/approval-choices.png" });
    }
    const button = page.getByRole("button", {
      name:
        decision === "deny"
          ? "Deny"
          : decision === "always_allow"
            ? "Always allow"
            : "Allow once",
      exact: true,
    });
    await page.setViewportSize({ width: 420, height: 360 });
    const surface = page.locator(".command-review-window");
    await surface.hover();
    await page.mouse.wheel(0, 2000);
    await expect
      .poll(() => surface.evaluate((element) => element.scrollTop))
      .toBeGreaterThan(0);
    await expect(button).toBeInViewport();
    await expect(
      page.getByRole("button", { name: "Deny", exact: true }),
    ).toBeInViewport();
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBe(420);
    await button.focus();
    const detailsBox = await full.boundingBox();
    const buttonBox = await button.boundingBox();
    expect(detailsBox!.y + detailsBox!.height).toBeLessThanOrEqual(
      buttonBox!.y,
    );
    await page.keyboard.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { reviewCalls: unknown[] }).reviewCalls,
        ),
      )
      .toEqual([
        { command: "command_review_context", args: {} },
        {
          command: "finish_command_review",
          args: { reviewId: "native-review-id", decision },
        },
      ]);
    await expect(
      page.getByRole("button", { name: "Applying decision…" }),
    ).toBeDisabled();
  });
}

test("stale native review fails closed without confirmation controls", async ({
  page,
}) => {
  await page.addInitScript(() =>
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async () => {
          throw new Error("expired");
        },
      },
    }),
  );
  await page.goto("/?surface=command-approval");
  await expect(page.getByRole("alert")).toContainText("no longer available");
  await expect(page.getByRole("button", { name: "Allow once" })).toHaveCount(0);
});

for (const commandContext of [
  null,
  {
    justification: "Check status",
    executable: "tool",
    arguments: ["[REDACTED]"],
    workingDirectory: "/work",
    redacted: true,
  },
]) {
  test(`one-time-only review does not offer broad consent: ${commandContext ? "redacted" : "non-command"}`, async ({
    page,
  }) => {
    await page.addInitScript(
      (context) =>
        Object.assign(window, {
          __TAURI_INTERNALS__: {
            invoke: async () => ({
              reviewId: "review",
              target: "Managed Local",
              commandContext: context,
              action: "network.http",
              resource: "https://example.com",
              reason: "Approval required",
              canRemember: false,
            }),
          },
        }),
      commandContext,
    );
    await page.goto("/?surface=command-approval");
    await expect(
      page.getByRole("button", { name: "Allow once", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Always allow", exact: true }),
    ).toHaveCount(0);
    const scan = await new AxeBuilder({ page }).analyze();
    expect(
      scan.violations.filter((item) =>
        ["critical", "serious"].includes(item.impact ?? ""),
      ),
    ).toEqual([]);
  });
}

test("workspace Access settings clear remembered commands", async ({
  page,
}) => {
  await page.addInitScript(() => {
    let count = 3;
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (command: string) => {
          if (command === "remembered_command_count") return count;
          if (command === "clear_remembered_commands") {
            count = 0;
            return;
          }
          if (command === "desktop_release_channel") return "development";
          throw new Error("Unexpected native command");
        },
      },
    });
  });
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Access", exact: true })
    .click();
  await expect(
    page.getByText("3 remembered commands", { exact: true }),
  ).toBeVisible();
  const clear = page.getByRole("button", {
    name: "Clear remembered commands",
    exact: true,
  });
  await clear.click();
  await expect(
    page.getByText("0 remembered commands", { exact: true }),
  ).toBeVisible();
  await expect(clear).toBeDisabled();
});
