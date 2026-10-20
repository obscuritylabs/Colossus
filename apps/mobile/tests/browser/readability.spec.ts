import { expect, test } from "@playwright/test";
import { writeFile } from "node:fs/promises";

const appearances = ["black", "colossus", "neutral", "hacker", "light"];

test("refresh preserves reading position and sending follows the reply", async ({
  page,
}) => {
  await page.route("**/src/preview.ts*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body:
        (await response.text()) +
        `
const readingRequest = previewClient.request.bind(previewClient);
previewClient.request = async (operation) => {
  let refreshed = false;
  if(operation.kind === "thread" && Reflect.get(window,"__pauseMobileNextRead")) {
    Reflect.set(window,"__pauseMobileNextRead",false);
    Reflect.set(window,"__mobileReadPending",true);
    await new Promise(resolve => Reflect.set(window,"__releaseMobileRead",resolve));
    Reflect.set(window,"__mobileReadPending",false);
    refreshed = true;
  }
  const result = await readingRequest(operation);
  return refreshed ? {...result,thread:{...result.thread,title:"Reading position refreshed"}} : result;
};
`,
    });
  });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: /Prepare release notes/ }).click();
  for (let index = 0; index < 3; index++) {
    await page
      .getByRole("textbox", { name: "Message Colossus" })
      .fill(`Follow-up ${index}`);
    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect(
      page.getByText(`Follow-up ${index}`, { exact: true }),
    ).toBeVisible();
  }
  const body = page.locator(".mobile-conversation-body");
  await body.evaluate((element) => {
    element.addEventListener("scroll", () =>
      Reflect.set(window, "__mobileReadingScrollTop", element.scrollTop),
    );
  });
  await body.hover();
  await page.mouse.wheel(0, -1600);
  await expect
    .poll(() =>
      page.evaluate(() => Reflect.get(window, "__mobileReadingScrollTop")),
    )
    .toBe(0);
  await page.evaluate(() => Reflect.set(window, "__pauseMobileNextRead", true));
  await page.getByRole("button", { name: "Refresh conversation" }).click();
  await expect
    .poll(() => page.evaluate(() => Reflect.get(window, "__mobileReadPending")))
    .toBe(true);
  await expect(
    page.getByText("Release notes ready", { exact: true }),
  ).toBeVisible();
  expect(await body.evaluate((element) => element.scrollTop)).toBe(0);
  await page.evaluate(() => Reflect.get(window, "__releaseMobileRead")());
  await expect
    .poll(() => page.evaluate(() => Reflect.get(window, "__mobileReadPending")))
    .toBe(false);
  await expect(
    page.getByRole("heading", {
      name: "Reading position refreshed",
      exact: true,
    }),
  ).toBeVisible();
  expect(await body.evaluate((element) => element.scrollTop)).toBe(0);
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Return to latest work");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() =>
      body.evaluate(
        (element) =>
          element.scrollHeight - element.clientHeight - element.scrollTop,
      ),
    )
    .toBeLessThan(2);
});

for (const appearance of appearances) {
  test(`conversation stays readable and usable in ${appearance}`, async ({
    page,
  }) => {
    await page.addInitScript((appearance) => {
      localStorage.setItem(
        "colossus.mobile.theme",
        appearance === "light" ? "light" : "dark",
      );
      localStorage.setItem(
        "colossus.mobile.palette",
        appearance === "light" ? "black" : appearance,
      );
    }, appearance);
    await page.goto("/");
    await page
      .getByRole("button", { name: "Explore development preview" })
      .click();
    await page.getByRole("combobox", { name: "Project", exact: true }).click();
    const projectOption = page.getByRole("option", {
      name: "Engineering",
      exact: true,
    });
    expect((await projectOption.boundingBox())!.height).toBeGreaterThanOrEqual(
      44,
    );
    expect(
      await projectOption.evaluate((element) =>
        parseFloat(getComputedStyle(element).fontSize),
      ),
    ).toBeGreaterThanOrEqual(16);
    await projectOption.click();
    await page.getByRole("button", { name: /Prepare release notes/ }).click();
    await expect(
      page.getByText("Release notes ready", { exact: true }),
    ).toBeVisible();
    const composer = page.getByRole("form", { name: "Message Colossus" });
    const body = page.locator(".mobile-conversation-body");
    expect((await composer.boundingBox())!.height).toBeLessThan(140);
    expect((await body.boundingBox())!.height).toBeGreaterThan(390);
    await expect(page.getByText("Task queued.", { exact: false })).toHaveCount(
      0,
    );
    const disabledColors = await page
      .getByRole("button", { name: "Send", exact: true })
      .evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          color: style.color,
          background: style.backgroundColor,
          opacity: style.opacity,
        };
      });
    expect(disabledColors.opacity).toBe("1");
    await page
      .getByRole("textbox", { name: "Message Colossus" })
      .fill("Check the migration notes.");
    const send = page.getByRole("button", { name: "Send", exact: true });
    await expect(send).toBeEnabled();
    await expect(send).toBeInViewport({ ratio: 1 });
    const readability = await page.evaluate((disabledColors) => {
      const luminance = (color: string) => {
        const channels = color
          .match(/[\d.]+/g)!
          .slice(0, 3)
          .map(Number)
          .map((v) => {
            const c = v / 255;
            return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
          });
        return (
          channels[0]! * 0.2126 + channels[1]! * 0.7152 + channels[2]! * 0.0722
        );
      };
      const contrast = (foreground: string, background: string) => {
        const [a, b] = [luminance(foreground), luminance(background)].sort(
          (x, y) => y - x,
        );
        return (a! + 0.05) / (b! + 0.05);
      };
      const shell = getComputedStyle(document.querySelector(".mobile-shell")!);
      const canvas = getComputedStyle(document.body).backgroundColor;
      const send = getComputedStyle(
        document.querySelector(".shared-composer-send")!,
      );
      const metadata = getComputedStyle(
        document.querySelector(
          ".mobile-conversation-context .mobile-task-state",
        )!,
      );
      return {
        textSize: parseFloat(
          getComputedStyle(document.querySelector(".shared-message-body")!)
            .fontSize,
        ),
        inputSize: parseFloat(
          getComputedStyle(document.querySelector("textarea")!).fontSize,
        ),
        textContrast: contrast(shell.color, canvas),
        metadataContrast: contrast(metadata.color, canvas),
        sendContrast: contrast(send.color, send.backgroundColor),
        disabledSendContrast: contrast(
          disabledColors.color,
          disabledColors.background,
        ),
      };
    }, disabledColors);
    expect(readability.textSize).toBeGreaterThanOrEqual(16);
    expect(readability.inputSize).toBeGreaterThanOrEqual(16);
    expect(readability.textContrast).toBeGreaterThanOrEqual(4.5);
    expect(readability.metadataContrast).toBeGreaterThanOrEqual(4.5);
    expect(readability.sendContrast).toBeGreaterThanOrEqual(4.5);
    expect(readability.disabledSendContrast).toBeGreaterThanOrEqual(4.5);
    await writeFile(
      `test-results/readability-${appearance}.json`,
      JSON.stringify(
        {
          ...readability,
          composerHeight: (await composer.boundingBox())!.height,
          messageViewport: (await body.boundingBox())!.height,
        },
        null,
        2,
      ),
    );
    await page.screenshot({
      path: `test-results/readable-conversation-${appearance}.png`,
    });
    await page.getByRole("button", { name: "Task options · Execute" }).click();
    await expect(
      page.getByRole("dialog", { name: "Task options" }),
    ).toBeVisible();
    await expect(page.getByText("Mac studio · Connected")).toBeVisible();
    await page
      .locator(".mode-switch label")
      .filter({ hasText: "Plan" })
      .click();
    await page.screenshot({
      path: `test-results/readable-options-${appearance}.png`,
    });
    await page.keyboard.press("Escape");
    await expect(
      page.getByRole("button", { name: "Task options · Plan" }),
    ).toBeFocused();
    // A reduced viewport exercises composer geometry while the keyboard consumes space.
    await page.setViewportSize({ width: 390, height: 500 });
    await expect(send).toBeInViewport({ ratio: 1 });
    expect((await body.boundingBox())!.height).toBeGreaterThan(100);
    await expect
      .poll(async () => {
        const message = await page
          .locator(".shared-message-assistant .shared-message-body")
          .last()
          .boundingBox();
        const viewport = (await body.boundingBox())!;
        return message!.y + message!.height <= viewport.y + viewport.height + 1;
      })
      .toBe(true);
    await page.screenshot({
      path: `test-results/readable-short-${appearance}.png`,
    });
    await page.evaluate(() =>
      localStorage.setItem("colossus.mobile.text-size", "large"),
    );
    await page.reload();
    await page.setViewportSize({ width: 320, height: 568 });
    await page
      .getByRole("button", { name: "Explore development preview" })
      .click();
    await page.getByRole("button", { name: /Prepare release notes/ }).click();
    await expect(
      page.getByText("Release notes ready", { exact: true }),
    ).toBeVisible();
    await expect(send).toBeInViewport({ ratio: 1 });
    expect((await body.boundingBox())!.height).toBeGreaterThan(120);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBe(320);
    await page.screenshot({
      path: `test-results/readable-small-large-${appearance}.png`,
    });
    await page.getByRole("button", { name: "Task options · Execute" }).click();
    await expect(
      page.getByRole("button", { name: "Done", exact: true }),
    ).toBeInViewport({ ratio: 1 });
    await page
      .locator(".mode-switch label")
      .filter({ hasText: "Plan" })
      .click();
    await page.getByRole("button", { name: "Done", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Task options · Plan" }),
    ).toBeFocused();
  });
}

test("workspace names remain complete and options control the submitted task", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await page.getByRole("button", { name: "Task options · Execute" }).click();
  await expect(page.getByRole("radio", { name: /^Release / })).toBeVisible();
  await expect(page.getByText("Mac studio · Connected")).toBeVisible();
  await page.getByRole("radio", { name: /^Release / }).click();
  await page.locator(".mode-switch label").filter({ hasText: "Plan" }).click();
  await page.screenshot({ path: "test-results/readable-new-task-options.png" });
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Plan the release verification");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(
    page.getByRole("heading", {
      name: "Plan the release verification",
      exact: true,
    }),
  ).toBeVisible();
  const request = await page.evaluate(async () => {
    const fixturePath = "/src/preview.ts";
    const { previewClient } = await import(fixturePath);
    const { tasks } = await previewClient.request({
      kind: "tasks",
      project: "engineering",
    });
    return tasks.find(
      (t: { request: { input: { text: string }[] } }) =>
        t.request.input[0]?.text === "Plan the release verification",
    );
  });
  expect(request.node_id).toBe("node-2");
  expect(request.request.mode).toBe("plan");
  await expect(
    page.getByRole("button", { name: "Task options · Plan" }),
  ).toBeVisible();
  await expect(page.locator(".mobile-conversation-context")).toContainText(
    "Release",
  );
  await expect(page.locator(".mobile-conversation-context")).toContainText(
    "Completed",
  );
  await expect(page.getByText("Task queued.", { exact: false })).toHaveCount(0);
});
