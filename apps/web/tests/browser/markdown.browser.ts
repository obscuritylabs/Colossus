import { expect, test, type Page } from "@playwright/test";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

const chatUrl = "/projects/browser-project/chat/browser-thread";
const timestamp = "2026-10-16T10:01:00Z";
const source = [
  "def choose_joke():",
  '    message = "A deliberately long joke line that should stay inside the conversation at a compact width."',
  "",
  "    return message",
].join("\n");
const diagram = "flowchart LR\n  Request[Request] --> Review[Review]";
const response = `Here is the code.\n\n\`\`\`python\n${source}\n\`\`\`\n\n\`\`\`mermaid\n${diagram}\n\`\`\``;

async function recordClipboard(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          (window as unknown as { copiedText: string }).copiedText = text;
        },
      },
    });
  });
}

for (const appearance of appearanceCases) {
  test(`shared code and diagrams remain usable in compact ${appearance.label} Large chat`, async ({
    page,
  }) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      releasedMessages: [
        {
          message_id: "markdown-answer",
          role: "assistant",
          text: response,
          created_at: timestamp,
        },
      ],
    });
    await recordClipboard(page);
    await page.setViewportSize({ width: 390, height: 1200 });
    await selectAppearance(page, appearance);
    await page.goto(chatUrl);
    const message = page.locator('[data-aside-message-id="markdown-answer"]');
    const block = message.getByRole("region", {
      name: "python code block",
      exact: true,
    });
    await expect(block.locator(".markdown-code-language")).toHaveText("python");
    await expect(block.locator("pre code")).toHaveText(source);
    const keyword = block.locator(".syntax-keyword").first();
    await expect(keyword).toHaveText("def");
    await expect(block.locator("code [style]")).toHaveCount(0);
    expect(
      await keyword.evaluate((node) => getComputedStyle(node).color),
    ).not.toBe(
      await block
        .locator("code")
        .evaluate((node) => getComputedStyle(node).color),
    );

    const copy = block.getByRole("button", { name: "Copy code", exact: true });
    await copy.focus();
    await copy.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { copiedText: string }).copiedText,
        ),
      )
      .toBe(source);
    await expect(
      block.getByRole("button", { name: "Copied code", exact: true }),
    ).toBeFocused();
    const wrap = block.getByRole("button", { name: "Wrap code", exact: true });
    await wrap.focus();
    await wrap.press("Space");
    await expect(wrap).toHaveAttribute("aria-pressed", "true");
    await expect(block.locator("pre")).toHaveCSS("white-space", "pre-wrap");
    await expect(block.locator("pre code")).toHaveText(source);

    const mermaid = message.getByRole("region", {
      name: "mermaid code block",
      exact: true,
    });
    await expect(
      mermaid.getByRole("img", { name: "Mermaid diagram", exact: true }),
    ).toBeVisible();
    const fitWidth = (await mermaid
      .locator(".markdown-mermaid-diagram")
      .boundingBox())!.width;
    const actualSize = mermaid.getByRole("button", {
      name: "Actual size",
      exact: true,
    });
    await actualSize.focus();
    await actualSize.press("Enter");
    await expect
      .poll(
        async () =>
          (await mermaid.locator(".markdown-mermaid-diagram").boundingBox())!
            .width,
      )
      .toBeGreaterThan(fitWidth);
    await mermaid
      .getByRole("button", { name: "Fit diagram", exact: true })
      .click();
    await expect(mermaid.locator(".markdown-mermaid-diagram")).toContainText(
      "Request",
    );
    await expect(mermaid.locator(".markdown-mermaid-diagram")).toContainText(
      "Review",
    );
    expect(
      await mermaid.locator(".markdown-mermaid-diagram").evaluate((svg) => {
        const rectangles = [...svg.querySelectorAll("rect")].map((node) =>
          node.getBoundingClientRect(),
        );
        return [...svg.querySelectorAll("text")].every((node) => {
          const text = node.getBoundingClientRect();
          const x = text.left + text.width / 2;
          const y = text.top + text.height / 2;
          return rectangles.some(
            (rect) =>
              x >= rect.left &&
              x <= rect.right &&
              y >= rect.top &&
              y <= rect.bottom,
          );
        });
      }),
    ).toBe(true);
    await expect(
      mermaid.locator(
        "svg style, svg [style], svg a, svg foreignObject, svg image",
      ),
    ).toHaveCount(0);
    const viewSource = mermaid.getByRole("button", {
      name: "Source",
      exact: true,
    });
    await viewSource.focus();
    await viewSource.press("Enter");
    await expect(mermaid.locator("pre code")).toHaveText(diagram);
    await mermaid
      .getByRole("button", { name: "Copy code", exact: true })
      .click();
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { copiedText: string }).copiedText,
        ),
      )
      .toBe(diagram);
    await mermaid.getByRole("button", { name: "Preview", exact: true }).click();
    await expect(
      mermaid.getByRole("img", { name: "Mermaid diagram", exact: true }),
    ).toBeVisible();
    await expect(message.locator(".shared-message-usage")).toHaveCount(0);
    await message
      .getByRole("button", { name: "Copy Colossus response", exact: true })
      .click();
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { copiedText: string }).copiedText,
        ),
      )
      .toBe(response);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await message.screenshot({
      path: `output/playwright/markdown-${appearance.id}-large-compact.png`,
    });
    if (appearance.id === "dark") {
      await page.setViewportSize({ width: 1280, height: 1100 });
      await page
        .getByRole("button", { name: "Toggle app sidebar", exact: true })
        .focus();
      await page.mouse.move(0, 0);
      await expect(
        message.locator(".shared-message-footer .shared-message-copy"),
      ).toHaveCSS("opacity", "1");
      await message.screenshot({
        path: "output/playwright/markdown-dark-large-wide.png",
      });
    }
    fixture.verify();
  });
}

test("partial fenced code updates in place and usage belongs to the final response of its released run", async ({
  page,
}) => {
  const partial =
    'A streamed answer.\n\n```python\ndef choose_joke():\n    return "hel';
  const completed =
    'A streamed answer.\n\n```python\ndef choose_joke():\n    return "hello"\n```';
  const fixture = await browserFixture(page, {
    managedChat: true,
    running: true,
    releasedMessages: [
      {
        message_id: "earlier-answer",
        task_id: "browser-active-task",
        role: "assistant",
        text: "Earlier released response.",
        created_at: timestamp,
      },
      {
        message_id: "streamed-answer",
        task_id: "browser-active-task",
        role: "assistant",
        text: partial,
        created_at: "2026-10-16T10:02:00Z",
      },
    ],
    releasedUpdates: [
      {
        run_id: "other-run",
        task_id: "browser-active-task",
        sequence: 1,
        created_at: timestamp,
        update: {
          usage: {
            input_tokens: 9000,
            output_tokens: 9000,
            total_tokens: 18000,
          },
        },
      },
      {
        run_id: "browser-active-run",
        task_id: "browser-active-task",
        sequence: 2,
        created_at: timestamp,
        update: {
          usage: {
            input_tokens: 6564,
            output_tokens: 13,
            total_tokens: 6577,
            cached_input_tokens: 4096,
            reasoning_tokens: 3,
          },
        },
      },
    ],
  });
  await page.setViewportSize({ width: 1120, height: 1000 });
  await page.goto(chatUrl);
  const message = page.locator('[data-aside-message-id="streamed-answer"]');
  await expect(message.locator("article")).toHaveAttribute("aria-busy", "true");
  const code = message.getByRole("region", {
    name: "python code block",
    exact: true,
  });
  await expect(code.locator("pre code")).toContainText("def choose_joke()");
  await expect(code.locator(".syntax-keyword").first()).toHaveText("def");
  fixture.replaceReleasedMessage("streamed-answer", completed);
  await expect(code.locator("pre code")).toHaveText(
    'def choose_joke():\n    return "hello"',
  );
  await expect(message.locator(".markdown-code-block")).toHaveCount(1);
  await expect(
    page.locator(
      '[data-aside-message-id="earlier-answer"] .shared-message-usage',
    ),
  ).toHaveCount(0);
  const summary = message.locator(".shared-message-usage > summary");
  await expect(summary).toHaveAccessibleName(
    "Reported run usage: 6,577 tokens; 6,564 input, 13 output",
  );
  await summary.focus();
  await summary.press("Enter");
  await expect(message.locator(".shared-message-usage-details")).toContainText(
    "Cached input tokens4,096",
  );
  await expect(message.locator(".shared-message-usage-details")).toContainText(
    "Reasoning tokens3",
  );
  fixture.verify();
});
