import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";

test("Hosted text files use opaque references and retain a failed submission", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { managedChat: true });
  const uploads: Record<string, unknown>[] = [];
  const attempts: unknown[] = [];
  await page.route(
    "**/api/projects/browser-project/chat/threads",
    async (route) => {
      if (route.request().method() !== "POST") return route.fallback();
      attempts.push(route.request().postDataJSON());
      if (attempts.length === 1)
        return route.fulfill({ status: 503, json: { error: "storage" } });
      return route.fallback();
    },
  );
  await page.route(
    "**/api/projects/browser-project/hosted/files",
    async (route) => {
      uploads.push(route.request().postDataJSON());
      await route.fulfill({
        json: {
          artifact: {
            artifact_id: "owned-file",
            file_name: "report.csv",
            size_bytes: 19,
          },
        },
      });
    },
  );
  await page.goto("/chat?project=browser-project");
  await expect(
    page.getByRole("button", { name: "Attach file", exact: true }),
  ).toBeVisible();
  await page.getByLabel("Choose an attachment").setInputFiles({
    name: "report.csv",
    mimeType: "text/csv",
    buffer: Buffer.from("name,value\nrow,42\n"),
  });
  await expect(
    page.getByRole("button", { name: "Remove report.csv" }),
  ).toBeVisible();
  expect(uploads).toHaveLength(1);
  expect(uploads[0]).toMatchObject({
    file_name: "report.csv",
    media_type: "text/csv",
  });
  expect(Object.keys(uploads[0]!).sort()).toEqual([
    "content_base64",
    "file_name",
    "idempotency_key",
    "media_type",
  ]);
  await page
    .getByRole("textbox", { name: "Message Colossus", exact: true })
    .fill("Summarize this file");
  await page.getByRole("button", { name: "Send message", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "temporarily unavailable",
  );
  await expect(
    page.getByRole("button", { name: "Remove report.csv" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Send message", exact: true }).click();
  await expect.poll(() => fixture.submissions.length).toBe(1);
  expect(attempts).toHaveLength(2);
  expect(attempts[1]).toEqual(attempts[0]);
  expect(fixture.submissions[0]!.request.input).toEqual([
    { text: "Summarize this file" },
    { artifact: "owned-file" },
  ]);
  fixture.verify([
    "Failed to load resource: the server responded with a status of 503 (Service Unavailable)",
  ]);
});

test("Hosted attachment controls remain usable at narrow Large text sizes", async ({
  page,
}, testInfo) => {
  const fixture = await browserFixture(page, { managedChat: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const longName = `${"quarterly_report_".repeat(10)}.csv`;
  await page.route(
    "**/api/projects/browser-project/hosted/files",
    async (route) => {
      const body = route.request().postDataJSON();
      await route.fulfill({
        json: {
          artifact: {
            artifact_id: "long-name-file",
            file_name: body.file_name,
            size_bytes: 12,
          },
        },
      });
    },
  );
  await page.goto("/chat?project=browser-project");
  for (const appearance of appearanceCases) {
    await selectAppearance(page, appearance);
    await page.goto("/chat?project=browser-project");
    await page
      .getByLabel("Choose an attachment")
      .setInputFiles({
        name: longName,
        mimeType: "text/csv",
        buffer: Buffer.from("name,value\n"),
      });
    await expect(
      page.getByRole("button", { name: `Remove ${longName}`, exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Attach file", exact: true }),
    ).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    );
    expect(overflow).toBe(false);
    await page.screenshot({
      path: testInfo.outputPath(`hosted-files-${appearance.id}.png`),
      fullPage: true,
    });
  }
  await page.getByLabel("Choose an attachment").setInputFiles({
    name: "program.sh",
    mimeType: "text/plain",
    buffer: Buffer.from("exit 0"),
  });
  await expect(page.getByRole("alert")).toContainText(
    "TXT, Markdown, CSV, or JSON",
  );
  fixture.verify();
});
