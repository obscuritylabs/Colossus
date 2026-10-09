import { readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { initialView } from "../../src/model.js";

const processes = readFileSync(
  new URL("../fixtures/process-tables.txt", import.meta.url),
  "utf8",
);
async function show(page: Page, text: string) {
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
    await expect(tables.first().getByRole("cell").first()).toHaveCSS(
      "text-align",
      "right",
    );
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
