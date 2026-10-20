import { expect, test, type Locator } from "@playwright/test";

const layouts = [
  {
    viewport: { width: 490, height: 325 },
    insets: { top: 8, left: 28, right: 12, bottom: 12 },
  },
  {
    viewport: { width: 844, height: 390 },
    insets: { top: 16, left: 66, right: 22, bottom: 24 },
  },
  {
    viewport: { width: 740, height: 490 },
    insets: { top: 16, left: 30, right: 14, bottom: 20 },
  },
];

test("an unavailable long conversation explains its disabled composer without repeatedly moving the reader", async ({
  page,
}) => {
  await page.route("**/src/preview.ts*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body:
        (await response.text()) +
        `
threads[2].can_continue = false;
messages.get("thread-2")[0].text += "\\n\\n" + Array.from({length:20}, (_,i) => "Retained history paragraph " + i + ": this conversation remains available to read.").join("\\n\\n");
Reflect.set(window,"__releaseRestrictedActivity",() => {
  activity.get("task-2").push({run_id:"run-2",sequence:5,created_at:new Date().toISOString(),update:{notice:{message:"Additional released context",reason:"preview.history"}}});
});
`,
    });
  });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: /Prepare release notes/ }).click();
  const body = page.locator(".mobile-conversation-body");
  const reason = page.getByText(
    "Continuation is unavailable for this conversation or your project permissions.",
    { exact: true },
  );
  await expect(reason).toBeInViewport({ ratio: 1 });
  await expect(
    page.getByRole("textbox", { name: "Message Colossus" }),
  ).toBeDisabled();
  await body.hover();
  await page.mouse.wheel(0, 10000);
  await expect
    .poll(() =>
      body.evaluate(
        (element) =>
          element.scrollHeight - element.scrollTop - element.clientHeight,
      ),
    )
    .toBeLessThan(2);
  await expect(reason).not.toBeInViewport();
  await page.evaluate(() =>
    Reflect.get(window, "__releaseRestrictedActivity")(),
  );
  await expect(
    page.getByText("Additional released context", { exact: true }),
  ).toHaveCount(1);
  await expect(reason).not.toBeInViewport();
  await expect
    .poll(() => body.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
});

test("Large running conversations retain history and all header actions at narrow portrait widths", async ({
  page,
}) => {
  await page.addInitScript(() =>
    localStorage.setItem("colossus.mobile.text-size", "large"),
  );
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page
    .getByRole("button", { name: /Investigate connection timeout/ })
    .click();
  const body = page.locator(".mobile-conversation-body");
  for (const viewport of [
    { width: 320, height: 568 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    await expect
      .poll(() =>
        body.evaluate((element) => {
          const style = getComputedStyle(element);
          const message = element.querySelector(".shared-message-body")!;
          return (
            (element.clientHeight -
              parseFloat(style.paddingTop) -
              parseFloat(style.paddingBottom)) /
            parseFloat(getComputedStyle(message).lineHeight)
          );
        }),
      )
      .toBeGreaterThanOrEqual(3);
    const heading = page.getByRole("heading", {
      name: "Investigate connection timeout",
      exact: true,
    });
    await expect(heading).toBeInViewport({ ratio: 1 });
    expect(
      await heading.evaluate(
        (element) => element.scrollHeight <= element.clientHeight + 1,
      ),
    ).toBe(true);
    for (const name of [
      "Back to work",
      "Session views",
      "Cancel task",
      "Refresh conversation",
      "Task options · Execute",
      "Send",
    ]) {
      const control = page.getByRole("button", { name, exact: true });
      await expect(control).toBeInViewport({ ratio: 1 });
      const bounds = (await control.boundingBox())!;
      expect(bounds.height).toBeGreaterThanOrEqual(44);
      expect(bounds.width).toBeGreaterThanOrEqual(44);
    }
    await body.hover();
    await page.mouse.wheel(0, -10000);
    await expect
      .poll(() => body.evaluate((element) => element.scrollTop))
      .toBe(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `test-results/portrait-running-${viewport.width}.png`,
    });
  }
});

for (const [state, index, title] of [
  ["running", 0, "Investigate connection timeout"],
  ["completed", 2, "Prepare release notes"],
] as const) {
  test(`short ${state} conversations keep readable history and reachable actions with Large text`, async ({
    page,
    browserName,
  }) => {
    test.skip(
      browserName !== "chromium",
      "Safe-area emulation uses Chromium CDP.",
    );
    const cdp = await page.context().newCDPSession(page);
    try {
      await cdp.send("Emulation.setSafeAreaInsetsOverride", {
        insets: layouts[0]!.insets,
      });
    } catch (failure) {
      await cdp.detach();
      test.skip(
        String(failure).includes("wasn't found"),
        "This Chromium version does not implement safe-area emulation.",
      );
      throw failure;
    }
    await page.addInitScript(() =>
      localStorage.setItem("colossus.mobile.text-size", "large"),
    );
    const fullTitle = `${title}: verify reconnect recovery and retained activity across every supported workspace`;
    await page.route("**/src/preview.ts*", async (route) => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        body:
          (await response.text()) +
          `
threads[${index}].title = ${JSON.stringify(fullTitle)};
messages.get("thread-${index}")[0].text += "\\n\\n" + Array.from({length:12}, (_,i) => "History paragraph " + (i+1) + ": retain the complete diagnostic context while reading earlier work.").join("\\n\\n");
previewAuth.classification = {enabled:true,text:"TEST ONLY · Local preview"};
const shortHeightRequest = previewClient.request.bind(previewClient);
previewClient.request = async operation => {
  if(!Reflect.get(window,"__shortHeightOperations")) Reflect.set(window,"__shortHeightOperations",[]);
  Reflect.get(window,"__shortHeightOperations").push(structuredClone(operation));
  return shortHeightRequest(operation);
};
`,
      });
    });
    try {
      await page.goto("/");
      await page
        .getByRole("button", { name: "Explore development preview" })
        .click();
      await page.getByRole("button", { name: new RegExp(title) }).click();
      const body = page.locator(".mobile-conversation-body");
      const input = page.getByRole("textbox", {
        name: "Message Colossus",
        exact: true,
      });
      await input.fill("Continue checking the short landscape layout");
      for (const { viewport, insets } of layouts) {
        await page.setViewportSize(viewport);
        await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets });
        await expect
          .poll(async () =>
            body.evaluate((element) => {
              const style = getComputedStyle(element);
              const message = element.querySelector(".shared-message-body")!;
              return (
                (element.clientHeight -
                  parseFloat(style.paddingTop) -
                  parseFloat(style.paddingBottom)) /
                parseFloat(getComputedStyle(message).lineHeight)
              );
            }),
          )
          .toBeGreaterThanOrEqual(3);
        const safeTarget = async (control: Locator) => {
          await expect(control).toBeInViewport({ ratio: 1 });
          const bounds = (await control.boundingBox())!;
          expect(bounds.height).toBeGreaterThanOrEqual(44);
          expect(bounds.width).toBeGreaterThanOrEqual(44);
          expect(bounds.x).toBeGreaterThanOrEqual(insets.left - 1);
          expect(bounds.x + bounds.width).toBeLessThanOrEqual(
            viewport.width - insets.right + 1,
          );
          expect(bounds.y).toBeGreaterThanOrEqual(insets.top - 1);
          expect(bounds.y + bounds.height).toBeLessThanOrEqual(
            viewport.height - insets.bottom + 1,
          );
        };
        await safeTarget(input);
        await safeTarget(
          page.getByRole("button", { name: "Send", exact: true }),
        );
        await safeTarget(
          page.getByRole("button", { name: "Back to work", exact: true }),
        );
        await safeTarget(
          page.getByRole("combobox", { name: "Project", exact: true }),
        );
        for (const name of ["Work", "Fleet", "Inbox", "Settings"]) {
          await safeTarget(page.getByRole("button", { name, exact: true }));
        }
        if (state === "running")
          await safeTarget(
            page.getByRole("button", { name: "Cancel task", exact: true }),
          );
        else
          await expect(
            page.getByRole("button", { name: "Cancel task", exact: true }),
          ).toHaveCount(0);
        await expect(
          page.getByText("Development preview · Sample data", { exact: true }),
        ).toBeInViewport({ ratio: 1 });
        await expect(
          page.getByText("TEST ONLY · Local preview", { exact: true }),
        ).toBeInViewport({ ratio: 1 });
        await body.hover();
        await page.mouse.wheel(0, -10000);
        await expect
          .poll(() => body.evaluate((element) => element.scrollTop))
          .toBe(0);
        const heading = page.getByRole("heading", {
          name: fullTitle,
          exact: true,
        });
        await expect(heading).toBeInViewport();
        // A long title may itself need scrolling, but its text must wrap in full.
        expect(
          await heading.evaluate(
            (element) => element.scrollHeight <= element.clientHeight + 1,
          ),
        ).toBe(true);
        await body.hover();
        await page.mouse.wheel(0, 10000);
        await expect
          .poll(() =>
            body.evaluate(
              (element) =>
                element.scrollHeight - element.scrollTop - element.clientHeight,
            ),
          )
          .toBeLessThan(2);
        const options = page.getByRole("button", {
          name: "Task options · Execute",
          exact: true,
        });
        await safeTarget(options);
        await options.click();
        const done = page.getByRole("button", { name: "Done", exact: true });
        await safeTarget(done);
        await done.click();
        await expect(options).toBeFocused();
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
        await page.screenshot({
          path: `test-results/short-${state}-${viewport.width}.png`,
        });
      }
      if (state === "running") {
        await page
          .getByRole("button", { name: "Cancel task", exact: true })
          .click();
        await expect(
          page.locator(".mobile-conversation-context"),
        ).toContainText("Cancelled");
        expect(
          await page.evaluate(() =>
            Reflect.get(window, "__shortHeightOperations").filter(
              (operation: { kind: string }) => operation.kind === "cancel",
            ),
          ),
        ).toMatchObject([{ project: "engineering", task: "task-0" }]);
      }
      await page.getByRole("button", { name: "Send", exact: true }).click();
      await expect(
        page.getByText("Continue checking the short landscape layout", {
          exact: true,
        }),
      ).toBeVisible();
      expect(
        await page.evaluate(() =>
          Reflect.get(window, "__shortHeightOperations").filter(
            (operation: { kind: string }) => operation.kind === "send_message",
          ),
        ),
      ).toMatchObject([
        {
          project: "engineering",
          thread: `thread-${index}`,
          request: {
            input: [{ text: "Continue checking the short landscape layout" }],
          },
        },
      ]);
    } finally {
      await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: {} });
      await cdp.detach();
    }
  });
}
