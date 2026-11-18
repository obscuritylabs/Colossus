import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";

const base = "/api/projects/browser-project/threads/browser-thread";
const older = "Older canonical message outside the loaded history.";
const large = `東京 🦀 ${"Released text. ".repeat(6000)}`;

test("delayed copy preparation requires a fresh activated Copy click", async ({
  page,
}) => {
  const fixture = await browserFixture(page, {
    managedChat: true,
    permissions: ["read"],
  });
  await page.addInitScript(() => {
    const writes: { text: string; activated: boolean }[] = [];
    Object.assign(window, { transcriptClipboardWrites: writes });
    let activated = false;
    document.addEventListener(
      "click",
      () => {
        activated = true;
        setTimeout(() => {
          activated = false;
        }, 0);
      },
      true,
    );
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText(text: string) {
          writes.push({ text, activated });
          return activated
            ? Promise.resolve()
            : Promise.reject(
                new DOMException(
                  "A fresh click is required",
                  "NotAllowedError",
                ),
              );
        },
      },
    });
  });
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  const transcript =
    "# Canonical snapshot\n\nPrepared before this copy click.\n";
  await page.route(`**${base}/transcript/export`, async (route) => {
    await pending;
    return route.fulfill({ contentType: "text/markdown", body: transcript });
  });
  await page.goto("/projects/browser-project/chat/browser-thread");
  await page
    .getByRole("button", { name: "Conversation actions", exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: "Export conversation…", exact: true })
    .click();
  const dialog = page.getByRole("dialog", {
    name: "Export conversation",
    exact: true,
  });
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Cancel export", exact: true }),
  ).toBeVisible();
  const writes = () =>
    page.evaluate(
      () =>
        (
          window as Window & {
            transcriptClipboardWrites: { text: string; activated: boolean }[];
          }
        ).transcriptClipboardWrites,
    );
  expect(await writes()).toEqual([]);
  release();
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toBeEnabled();
  expect(await writes()).toEqual([]);
  await dialog
    .getByRole("button", { name: "Copy transcript", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText("Transcript copied.");
  expect(await writes()).toEqual([{ text: transcript, activated: true }]);
  fixture.verify();
});

for (const host of ["Chat", "Fleet", "Hosted"] as const) {
  test(`${host} saves complete server transcript in all formats with read access`, async ({
    page,
  }, testInfo) => {
    const fixture = await browserFixture(page, {
      managedChat: host !== "Fleet",
      hostedChat: host === "Hosted",
      permissions: ["read"],
      releasedMessages: [
        {
          message_id: "loaded",
          role: "assistant",
          text: "Only this recent message is loaded.",
          created_at: "2026-11-18T12:00:00Z",
        },
      ],
    });
    const requests: string[] = [];
    await page.route(`**${base}/transcript/export`, async (route) => {
      expect(route.request().method()).toBe("POST");
      expect(route.request().headers()["x-colossus-csrf"]).toBe("1");
      const { format } = route.request().postDataJSON() as {
        format: "markdown" | "text" | "json";
      };
      requests.push(format);
      const extension = { markdown: "md", text: "txt", json: "json" }[format];
      const contentType = {
        markdown: "text/markdown",
        text: "text/plain",
        json: "application/json",
      }[format];
      const body =
        format === "json"
          ? JSON.stringify({
              schema_version: 1,
              messages: [
                { role: "user", parts: [{ type: "text", text: older }] },
                { role: "assistant", parts: [{ type: "text", text: large }] },
              ],
              complete: true,
            })
          : `${format === "markdown" ? "# " : ""}Conversation\n\nUser\n${older}\n\nAssistant\n${large}\n\nEnd of snapshot.\n`;
      return route.fulfill({
        contentType,
        headers: {
          "Content-Disposition": `attachment; filename*=UTF-8''%E6%9D%B1%E4%BA%AC-session.${extension}`,
        },
        body,
      });
    });
    await page.goto(
      host === "Fleet"
        ? "/projects/browser-project/threads/browser-thread"
        : "/projects/browser-project/chat/browser-thread",
    );
    await page
      .getByRole("button", { name: "Conversation actions", exact: true })
      .click();
    const action = page.getByRole("menuitem", {
      name: "Export conversation…",
      exact: true,
    });
    await action.focus();
    await action.press("Enter");
    const dialog = page.getByRole("dialog", {
      name: "Export conversation",
      exact: true,
    });
    await expect(dialog).toBeVisible();
    await expect(
      dialog.getByRole("combobox", { name: "Format", exact: true }),
    ).toContainText("Markdown");
    expect(requests).toEqual([]);
    for (const format of ["markdown", "text", "json"] as const) {
      if (format !== "markdown") {
        await dialog
          .getByRole("combobox", { name: "Format", exact: true })
          .click();
        await page
          .getByRole("option", {
            name: format === "text" ? "Plain text (.txt)" : "JSON (.json)",
            exact: true,
          })
          .click();
      }
      const pending = page.waitForEvent("download");
      await dialog
        .getByRole("button", { name: "Save transcript", exact: true })
        .click();
      const download = await pending;
      expect(download.suggestedFilename()).toBe(
        `東京-session.${{ markdown: "md", text: "txt", json: "json" }[format]}`,
      );
      const file = testInfo.outputPath(`${host}-complete.${format}`);
      await download.saveAs(file);
      const text = await readFile(file, "utf8");
      expect(text).toContain(older);
      expect(text).toContain(large);
      expect(Buffer.byteLength(text)).toBeGreaterThan(65_536);
      expect(text).not.toContain("Only this recent message is loaded");
      await expect(dialog.getByRole("status")).toHaveText("Download started.");
      if (format === "json") expect(JSON.parse(text).complete).toBe(true);
    }
    expect(requests).toEqual(["markdown", "text", "json"]);
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    await expect(
      page.getByRole("button", { name: "Conversation actions", exact: true }),
    ).toBeFocused();
    fixture.verify();
  });
}

for (const appearance of appearanceCases) {
  test(`export dialog remains keyboard usable at narrow Large text in ${appearance.label}`, async ({
    page,
  }, testInfo) => {
    const fixture = await browserFixture(page, {
      managedChat: true,
      permissions: ["read"],
    });
    await selectAppearance(page, appearance);
    await page.goto("/projects/browser-project/chat/browser-thread");
    await page
      .getByRole("button", { name: "Conversation actions", exact: true })
      .click();
    await page
      .getByRole("menuitem", { name: "Export conversation…", exact: true })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "Export conversation",
      exact: true,
    });
    await expect(dialog).toBeVisible();
    const bounds = await dialog.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.width).toBeLessThanOrEqual(390);
    expect(bounds!.height).toBeLessThanOrEqual(844);
    await dialog.getByRole("combobox", { name: "Format", exact: true }).focus();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(
      page.getByRole("button", { name: "Conversation actions", exact: true }),
    ).toBeFocused();
    await page
      .getByRole("button", { name: "Conversation actions", exact: true })
      .click();
    await page
      .getByRole("menuitem", { name: "Export conversation…", exact: true })
      .click();
    await page.screenshot({
      path: testInfo.outputPath(`export-${appearance.id}-large.png`),
      fullPage: true,
    });
    fixture.verify();
  });
}
