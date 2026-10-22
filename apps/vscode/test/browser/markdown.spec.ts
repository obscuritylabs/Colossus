import { readFileSync, writeFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import {
  initialView,
  type InspectionView,
  type WorkView,
} from "../../src/model.js";

const processes = readFileSync(
  new URL("../fixtures/process-tables.txt", import.meta.url),
  "utf8",
);
async function show(
  page: Page,
  text: string,
  overrides: Partial<WorkView> = {},
) {
  await page.addInitScript(() => {
    (
      window as unknown as { acquireVsCodeApi: () => unknown }
    ).acquireVsCodeApi = () => ({
      postMessage() {},
      getState() {},
      setState() {},
    });
  });
  await page.goto("/");
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    {
      ...initialView("sample"),
      connected: true,
      status: "Completed",
      messages: [{ id: "sample", role: "assistant", text }],
      ...overrides,
    },
  );
}

for (const width of [260, 430, 700]) {
  test(`process tables have headers, aligned cells, and contained scrolling at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1100 });
    await show(page, processes);
    const tables = page.getByRole("table");
    await expect(tables).toHaveCount(2);
    await expect(tables.first().getByRole("columnheader")).toHaveText([
      "PID",
      "CPU",
      "Memory",
      "Process",
    ]);
    await expect(tables.first().getByRole("row")).toHaveCount(5);
    await expect(tables.first().getByRole("columnheader").first()).toHaveCSS(
      "text-align",
      "right",
    );
    await expect(tables.first().getByRole("columnheader").last()).toHaveCSS(
      "text-align",
      "left",
    );
    await expect(tables.first().getByRole("cell").first()).toHaveCSS(
      "text-align",
      "right",
    );
    await expect(tables.first().locator("th[style], td[style]")).toHaveCount(0);
    await expect(tables.first().locator("td code")).toHaveText("colossus");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    const scroll = page
      .getByRole("region", { name: /Scrollable Markdown table/u })
      .first();
    await scroll.focus();
    await expect(scroll).toBeFocused();
    if (width === 260) {
      expect(
        await scroll.evaluate((el) => el.scrollWidth > el.clientWidth),
      ).toBe(true);
      await scroll.evaluate((el) => {
        el.scrollLeft = el.scrollWidth;
      });
      expect(await scroll.evaluate((el) => el.scrollLeft > 0)).toBe(true);
    }
    expect(
      (
        await new AxeBuilder({ page })
          .include(".markdown-table-scroll")
          .analyze()
      ).violations,
    ).toEqual([]);
    await page.screenshot({ path: `artifacts/process-tables-${width}.png` });
  });
}

test("table cells preserve formatting and escaped pipes without loading or executing model markup", async ({
  page,
}) => {
  const outbound: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4312"))
      outbound.push(request.url());
  });
  await show(
    page,
    `| Name | Result |\n| --- | --- |\n| **strong** | \\| pipe |\n| [link](https://example.invalid/secret) | <img src="https://example.invalid/steal" onerror="alert(1)"> |\n| image | ![sample](https://example.invalid/image) |`,
  );
  await expect(page.getByRole("table")).toHaveCount(1);
  await expect(page.locator("table strong")).toHaveText("strong");
  await expect(
    page.getByRole("cell", { name: "| pipe", exact: true }),
  ).toBeVisible();
  await expect(page.locator("#messages a, #messages script")).toHaveCount(0);
  // The assistant avatar is the only image; generated remote images stay inert.
  await expect(page.locator("#messages img")).toHaveCount(1);
  await expect(
    page.getByText('onerror="alert(1)"', { exact: false }),
  ).toHaveCount(0);
  expect(outbound).toEqual([]);
});

test("oversized and over-budget tables use the bounded plain-text fallback", async ({
  page,
}) => {
  const header = `|${Array(400).fill("a").join("|")}|`;
  const separator = `|${Array(400).fill("---").join("|")}|`;
  const text = `${header}\n${separator}\n${header}`;
  expect(text.length).toBeLessThan(16_384);
  await show(page, text);
  await expect(page.locator(".shared-markdown pre code")).toHaveText(text);
  await expect(page.getByRole("table")).toHaveCount(0);
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    {
      ...initialView("sample"),
      connected: true,
      messages: [
        { id: "large", role: "assistant", text: processes.repeat(40) },
      ],
    },
  );
  await expect(page.locator(".markdown-plain-text")).toHaveText(
    processes.repeat(40),
  );
  await expect(page.getByRole("table")).toHaveCount(0);
});

for (const width of [260, 430, 700]) {
  test(`shared highlighted code, diagrams, copy, and usage work under the strict webview CSP at ${width}px`, async ({
    page,
  }) => {
    const outbound: string[] = [];
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error")
        errors.push(
          `${message.text()} at ${JSON.stringify(message.location())}`,
        );
    });
    page.on("request", (request) => {
      if (!request.url().startsWith("http://127.0.0.1:4312"))
        outbound.push(request.url());
    });
    await page.addInitScript(() => {
      const captures = window as unknown as {
        copiedText: string;
        policyViolations: string[];
      };
      captures.policyViolations = [];
      document.addEventListener("securitypolicyviolation", (event) =>
        captures.policyViolations.push(
          `${event.violatedDirective} at ${event.sourceFile}:${event.lineNumber}:${event.columnNumber}`,
        ),
      );
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: {
          writeText: async (text: string) => {
            captures.copiedText = text;
          },
        },
      });
    });
    await page.setViewportSize({ width, height: 1100 });
    const code =
      'def greet():\n    # Preserve whitespace and the exact copied source.\n    return "hello from the shared renderer"';
    const diagram = "flowchart LR\n  Request[Request] --> Review[Review]";
    const text = `\`\`\`python\n${code}\n\`\`\`\n\n\`\`\`mermaid\n${diagram}\n\`\`\``;
    await show(page, text, {
      messages: [
        {
          id: "sample",
          role: "assistant",
          text,
          usage: {
            inputTokens: "6564",
            outputTokens: "13",
            totalTokens: "6577",
            cachedInputTokens: "4096",
            reasoningTokens: "3",
          },
        },
      ],
    });
    const block = page.getByRole("region", {
      name: "python code block",
      exact: true,
    });
    await expect(block.locator("pre code")).toHaveText(code);
    await expect(block.locator(".syntax-keyword").first()).toHaveText("def");
    await expect(block.locator("code [style]")).toHaveCount(0);
    const copy = block.getByRole("button", { name: "Copy code", exact: true });
    await copy.focus();
    await copy.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { copiedText: string }).copiedText,
        ),
      )
      .toBe(code);
    const wrap = block.getByRole("button", { name: "Wrap code", exact: true });
    await wrap.focus();
    await wrap.press("Space");
    await expect(wrap).toHaveAttribute("aria-pressed", "true");
    await expect(block.locator("pre")).toHaveCSS("white-space", "pre-wrap");
    const preview = page.getByRole("img", {
      name: "Mermaid diagram",
      exact: true,
    });
    try {
      await expect(preview).toBeVisible();
    } catch (error) {
      const diagnosticPath = test
        .info()
        .outputPath("markdown-rendering-diagnostics.json");
      writeFileSync(
        diagnosticPath,
        JSON.stringify(
          {
            errors,
            policyViolations: await page.evaluate(
              () =>
                (window as unknown as { policyViolations: string[] })
                  .policyViolations,
            ),
          },
          null,
          2,
        ),
      );
      await test.info().attach("markdown-rendering-diagnostics.json", {
        contentType: "application/json",
        path: diagnosticPath,
      });
      throw error;
    }
    await expect(preview).toContainText("Request");
    await expect(
      page.locator(
        ".markdown-mermaid-preview style, .markdown-mermaid-preview [style], .markdown-mermaid-preview a, .markdown-mermaid-preview foreignObject",
      ),
    ).toHaveCount(0);
    const summary = page.locator(".shared-message-usage > summary");
    await expect(summary).toHaveAccessibleName(
      "Reported run usage: 6,577 tokens; 6,564 input, 13 output",
    );
    await summary.focus();
    await summary.press("Enter");
    await expect(page.locator(".shared-message-usage-details")).toContainText(
      "Cached input tokens4,096",
    );
    await page
      .getByRole("button", { name: "Copy Colossus response", exact: true })
      .click();
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { copiedText: string }).copiedText,
        ),
      )
      .toBe(text);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    const violations = await page.evaluate(
      () =>
        (window as unknown as { policyViolations: string[] }).policyViolations,
    );
    if (violations.length > 0) {
      const diagnosticPath = test
        .info()
        .outputPath("markdown-rendering-diagnostics.json");
      writeFileSync(
        diagnosticPath,
        JSON.stringify({ errors, policyViolations: violations }, null, 2),
      );
      await test.info().attach("markdown-rendering-diagnostics.json", {
        contentType: "application/json",
        path: diagnosticPath,
      });
    }
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { policyViolations: string[] })
            .policyViolations,
      ),
    ).toEqual([]);
    expect(errors).toEqual([]);
    expect(outbound).toEqual([]);
    expect(
      (await new AxeBuilder({ page }).include(".markdown-code-block").analyze())
        .violations,
    ).toEqual([]);
    await page.screenshot({
      path: `artifacts/markdown-code-diagram-${width}.png`,
    });
  });
}

test("hostile Mermaid directives, links, and images stay inert in generated messages", async ({
  page,
}) => {
  const outbound: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4312"))
      outbound.push(request.url());
  });
  const hostile = [
    'flowchart LR\n  A --> B\n  click A "https://example.invalid/steal"',
    '%%{init: {"securityLevel": "loose"}}%%\nflowchart LR\n A --> B',
    'flowchart LR\n  A[<img src="https://example.invalid/image">] --> B',
  ];
  const text = [
    "[Private link](https://example.invalid/private)",
    "![Generated image](https://example.invalid/image)",
    '<img src="https://example.invalid/raw" onerror="alert(1)">',
    ...hostile.map((source) => `\`\`\`mermaid\n${source}\n\`\`\``),
  ].join("\n\n");
  await show(page, text);
  await expect(page.locator(".markdown-code-block")).toHaveCount(
    hostile.length,
  );
  await expect(
    page.getByText(
      "Diagram preview unavailable for this source. View the code instead.",
      { exact: true },
    ),
  ).toHaveCount(hostile.length);
  await expect(
    page.locator("#messages a, #messages script, .markdown-mermaid-preview"),
  ).toHaveCount(0);
  await expect(page.locator("#messages img")).toHaveCount(1);
  await expect(
    page.getByText("Image omitted: Generated image", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText('onerror="alert(1)"', { exact: false }),
  ).toHaveCount(0);
  expect(outbound).toEqual([]);
});

test("streamed Mermaid source waits for completion and final syntax renders without replacing the message", async ({
  page,
}) => {
  const initial = initialView("sample");
  const run: WorkView["runs"][number] = {
    id: "streaming-run",
    sessionId: "sample-session",
    title: "Streaming response",
    role: "primary",
    mode: "execute",
    status: "running",
    createdAt: "2026-10-16T10:00:00Z",
    updatedAt: "2026-10-16T10:00:00Z",
    startedAt: "2026-10-16T10:00:00Z",
    finishedAt: "",
    sequence: "1",
    pendingInteractions: 0,
  };
  const partial = "```mermaid\nflowchart LR\n A[Request] -->";
  await show(page, partial, {
    runs: [run],
    messages: [
      { id: "stream", runId: run.id, role: "assistant", text: partial },
    ],
  });
  const message = page.locator(
    ".shared-conversation-entry[data-role='assistant']",
  );
  await expect(message).toHaveAttribute("aria-busy", "true");
  await expect(
    page.getByText("Diagram preview will appear when the code is complete.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("img", { name: "Mermaid diagram", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    {
      ...initial,
      connected: true,
      status: "Completed",
      runs: [{ ...run, status: "completed" }],
      messages: [
        {
          id: "stream",
          runId: run.id,
          role: "assistant" as const,
          text: "```mermaid\nflowchart LR\n A[Request] --> B[Review]\n```",
        },
      ],
    },
  );
  await expect(message).toHaveAttribute("aria-busy", "false");
  await expect(message).toHaveCount(1);
  await expect(
    page.getByRole("img", { name: "Mermaid diagram", exact: true }),
  ).toBeVisible();
});

test("every supported offline diagram renders without style exceptions or network access", async ({
  page,
}) => {
  const errors: string[] = [];
  const outbound: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error")
      errors.push(`${message.text()} at ${JSON.stringify(message.location())}`);
  });
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4312"))
      outbound.push(request.url());
  });
  await page.addInitScript(() => {
    const captured = window as unknown as { policyViolations: string[] };
    captured.policyViolations = [];
    document.addEventListener("securitypolicyviolation", (event) =>
      captured.policyViolations.push(
        `${event.violatedDirective} at ${event.sourceFile}:${event.lineNumber}:${event.columnNumber}`,
      ),
    );
  });
  await page.setViewportSize({ width: 700, height: 1100 });
  const sources = [
    "flowchart LR\n A[User] --> B[Runtime]",
    "sequenceDiagram\n User->>Runtime: Start\n Runtime-->>User: Done",
    "stateDiagram-v2\n [*] --> Ready\n Ready --> Running",
    "classDiagram\n Animal <|-- Duck",
    "erDiagram\n USER ||--o{ CHAT : owns",
    "gantt\n dateFormat YYYY-MM-DD\n Work :a1, 2026-10-21, 1d",
    'pie title Outcomes\n "Ready" : 3\n "Running" : 1',
  ];
  await show(
    page,
    sources.map((source) => `\`\`\`mermaid\n${source}\n\`\`\``).join("\n\n"),
  );
  const blocks = page.getByRole("region", {
    name: "mermaid code block",
    exact: true,
  });
  await expect(blocks).toHaveCount(sources.length);
  for (let index = 0; index < sources.length; index++) {
    await expect(
      blocks
        .nth(index)
        .getByRole("img", { name: "Mermaid diagram", exact: true }),
    ).toBeVisible();
    await expect(
      blocks.nth(index).locator(".markdown-mermaid-diagram"),
    ).toHaveText(/\S/u);
  }
  const policyViolations = await page.evaluate(
    () =>
      (window as unknown as { policyViolations: string[] }).policyViolations,
  );
  if (policyViolations.length || errors.length) {
    const diagnosticPath = test
      .info()
      .outputPath("markdown-rendering-diagnostics.json");
    writeFileSync(
      diagnosticPath,
      JSON.stringify({ errors, policyViolations }, null, 2),
    );
    await test.info().attach("markdown-rendering-diagnostics.json", {
      contentType: "application/json",
      path: diagnosticPath,
    });
  }
  expect(policyViolations).toEqual([]);
  expect(errors).toEqual([]);
  expect(outbound).toEqual([]);
});

test("inspector output keeps Markdown mounted for highlighting, diagrams and controls until replacement or disposal", async ({
  page,
}) => {
  const errors: string[] = [];
  const outbound: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4312"))
      outbound.push(request.url());
  });
  await page.addInitScript(() => {
    const captured = window as unknown as {
      copiedText: string;
      policyViolations: string[];
      acquireVsCodeApi: () => unknown;
    };
    captured.acquireVsCodeApi = () => ({ postMessage() {} });
    captured.policyViolations = [];
    document.addEventListener("securitypolicyviolation", (event) =>
      captured.policyViolations.push(event.violatedDirective),
    );
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          captured.copiedText = text;
        },
      },
    });
  });
  await page.goto("/inspector");
  const code = 'def inspect():\n    return "saved output"';
  const diagram = "flowchart LR\n A[Saved output] --> B[Review]";
  const view: InspectionView = {
    run: {
      id: "inspector-run",
      sessionId: "inspector-session",
      title: "Inspect saved output",
      role: "primary",
      mode: "execute",
      status: "completed",
      createdAt: "2026-10-21T10:00:00Z",
      updatedAt: "2026-10-21T10:00:01Z",
      startedAt: "2026-10-21T10:00:00Z",
      finishedAt: "2026-10-21T10:00:01Z",
      sequence: "3",
      pendingInteractions: 0,
    },
    output: [
      `\`\`\`python\n${code}\n\`\`\``,
      `\`\`\`mermaid\n${diagram}\n\`\`\``,
      "[Generated link](https://example.invalid/private)",
      "![Generated image](https://example.invalid/image)",
    ].join("\n\n"),
    model: "fixture-model",
    provider: "fixture-provider",
    activities: [],
    activityState: "No activity",
    activityHasMore: false,
    observedAt: "2026-10-21T10:00:02Z",
  };
  const inspect = (next: InspectionView) =>
    page.evaluate(
      (value) =>
        window.postMessage(
          {
            type: "inspection",
            view: value,
            loading: false,
            connected: true,
            error: "",
          },
          "*",
        ),
      next,
    );
  await inspect(view);
  await page.getByRole("button", { name: "Output", exact: true }).click();
  const output = page.locator("#inspection-content");
  const block = output.getByRole("region", {
    name: "python code block",
    exact: true,
  });
  await expect(block.locator(".syntax-keyword").first()).toHaveText("def");
  await block.getByRole("button", { name: "Copy code", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { copiedText: string }).copiedText,
      ),
    )
    .toBe(code);
  const wrap = block.getByRole("button", { name: "Wrap code", exact: true });
  await wrap.click();
  await expect(wrap).toHaveAttribute("aria-pressed", "true");
  await expect(block.locator("pre")).toHaveCSS("white-space", "pre-wrap");
  const preview = output.getByRole("img", {
    name: "Mermaid diagram",
    exact: true,
  });
  await expect(preview).toBeVisible();
  const mermaid = output.getByRole("region", {
    name: "mermaid code block",
    exact: true,
  });
  await mermaid.getByRole("button", { name: "Source", exact: true }).click();
  await expect(mermaid.locator("pre code")).toHaveText(diagram);
  await mermaid.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(preview).toBeVisible();
  await expect(output.locator("a, img, script, foreignObject")).toHaveCount(0);

  // Refreshes of unchanged output retain the live controls; changing tabs
  // disposes the old root and remounts a fully interactive output preview.
  await inspect({ ...view, observedAt: "2026-10-21T10:00:03Z" });
  await expect(wrap).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await expect(output.locator(".markdown-code-block")).toHaveCount(0);
  await page.getByRole("button", { name: "Output", exact: true }).click();
  await expect(block.locator(".syntax-keyword").first()).toHaveText("def");
  await expect(wrap).toHaveAttribute("aria-pressed", "false");
  await expect(preview).toBeVisible();
  await page.screenshot({ path: "artifacts/inspector-markdown.png" });

  const replacement = 'def inspect():\n    return "replacement output"';
  await inspect({
    ...view,
    run: { ...view.run, id: "replacement-run" },
    output: `\`\`\`python\n${replacement}\n\`\`\``,
  });
  await expect(block.locator("pre code")).toHaveText(replacement);
  await expect(block.locator(".syntax-keyword").first()).toHaveText("def");
  await expect(preview).toHaveCount(0);
  await block.getByRole("button", { name: "Copy code", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { copiedText: string }).copiedText,
      ),
    )
    .toBe(replacement);
  await page.evaluate(() =>
    window.dispatchEvent(new PageTransitionEvent("pagehide")),
  );
  await expect(output.locator(".markdown-code-block")).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { policyViolations: string[] }).policyViolations,
    ),
  ).toEqual([]);
  expect(errors).toEqual([]);
  expect(outbound).toEqual([]);
});
