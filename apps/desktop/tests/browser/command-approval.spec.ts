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
    page.getByRole("button", { name: "Allow once", exact: true }),
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

for (const approved of [false, true]) {
  test(`native review document forwards only its one-use identity: ${approved ? "continue" : "deny"}`, async ({
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
    const button = page.getByRole("button", {
      name: approved ? "Continue to native confirmation" : "Deny",
      exact: true,
    });
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
          args: { reviewId: "native-review-id", approved },
        },
      ]);
    await expect(
      page.getByRole("button", { name: "Awaiting native confirmation…" }),
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
  await expect(
    page.getByRole("button", { name: "Continue to native confirmation" }),
  ).toHaveCount(0);
});
