import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function installExportBridge(page: Page) {
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      transcriptFixture: {
        requests: Record<string, unknown>[];
        clipboard: string[];
        clipboardAttempts: number;
        currentClick: boolean;
        defer: boolean;
        deliverCopyAfterCancel: boolean;
        error: string;
        finish: (() => void) | null;
      };
    };
    const fixture = {
      requests: [] as Record<string, unknown>[],
      clipboard: [] as string[],
      clipboardAttempts: 0,
      currentClick: false,
      defer: false,
      deliverCopyAfterCancel: false,
      error: "",
      finish: null as (() => void) | null,
    };
    host.transcriptFixture = fixture;
    document.addEventListener(
      "click",
      () => {
        fixture.currentClick = true;
      },
      true,
    );
    document.addEventListener("click", () => {
      fixture.currentClick = false;
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: (text: string) => {
          fixture.clipboardAttempts += 1;
          if (!fixture.currentClick)
            throw new DOMException(
              "A current click is required.",
              "NotAllowedError",
            );
          fixture.clipboard.push(text);
          return Promise.resolve();
        },
      },
    });
    let callback = 0;
    host.__TAURI_INTERNALS__ = {
      transformCallback: () => ++callback,
      unregisterCallback: () => undefined,
      invoke: async (
        command: string,
        args: {
          targetId?: string;
          request?: Record<string, unknown>;
          onProgress?: { onmessage: (value: unknown) => void };
        },
      ) => {
        if (command !== "export_conversation")
          return command === "list_setup_packages" ? [] : null;
        const request = args.request!;
        fixture.requests.push({ targetId: args.targetId, ...request });
        if (request.action === "cancel") {
          if (!fixture.deliverCopyAfterCancel) fixture.finish?.();
          return { status: "cancelled", messages: 0 };
        }
        args.onProgress?.onmessage({
          phase: "reading",
          pagesRead: 16,
          messagesRead: 31,
        });
        if (fixture.defer) {
          await new Promise<void>((resolve) => {
            fixture.finish = resolve;
          });
          if (
            !fixture.deliverCopyAfterCancel &&
            fixture.requests.some(
              (item) =>
                item.action === "cancel" &&
                item.operationId === request.operationId,
            )
          )
            return { status: "cancelled", messages: 0 };
        }
        if (fixture.error)
          throw {
            code: "export_incomplete",
            message: fixture.error,
            retryable: true,
            outcomeUnknown: false,
            violations: [],
          };
        return request.destination === "copy"
          ? {
              status: "copy",
              messages: 31,
              text: "# Canonical transcript\n\nOlder retained turn outside the loaded renderer cache\n",
            }
          : { status: "saved", messages: 31 };
      },
    };
  });
}

async function openExport(page: Page) {
  const trigger = page.getByRole("button", {
    name: "Conversation actions",
    exact: true,
  });
  await trigger.focus();
  await page.keyboard.press("ArrowDown");
  await page
    .getByRole("menuitem", { name: "Export conversation…", exact: true })
    .press("Enter");
  return page.getByRole("dialog", { name: "Export conversation", exact: true });
}

test("manual export defaults to Markdown, sends only native identities, supports formats/copy and restores focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  expect(
    await page.evaluate(() => (window as any).transcriptFixture.requests),
  ).toEqual([]);
  const dialog = await openExport(page);
  await expect(
    dialog.getByRole("combobox", { name: "Format", exact: true }),
  ).toContainText("Markdown (.md)");
  await dialog
    .getByRole("button", { name: "Save transcript…", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Conversation saved (31 messages).",
  );
  await dialog.getByRole("combobox", { name: "Format", exact: true }).click();
  await page.getByRole("option", { name: "JSON (.json)", exact: true }).click();
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Transcript ready (31 messages). Choose Copy transcript to copy it.",
  );
  expect(
    await page.evaluate(
      () => (window as any).transcriptFixture.clipboardAttempts,
    ),
  ).toBe(0);
  await dialog
    .getByRole("button", { name: "Copy transcript", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Transcript copied (31 messages).",
  );
  const data = await page.evaluate(() => (window as any).transcriptFixture);
  expect(data.requests).toHaveLength(2);
  expect(data.requests[0]).toMatchObject({
    action: "start",
    format: "markdown",
    destination: "save",
    targetId: "fixture-managed-local",
  });
  expect(data.requests[1]).toMatchObject({
    action: "start",
    format: "json",
    destination: "copy",
  });
  expect(data.requests[0].runId).toBeTruthy();
  expect(Object.keys(data.requests[0]).sort()).toEqual(
    [
      "action",
      "destination",
      "discoveryProof",
      "format",
      "operationId",
      "runId",
      "targetId",
    ].sort(),
  );
  expect(data.clipboard[0]).toContain(
    "Older retained turn outside the loaded renderer cache",
  );
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Conversation actions", exact: true }),
  ).toBeFocused();
});

test("cancellation and failed snapshots never show a successful partial export", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  await page.evaluate(() => {
    (window as any).transcriptFixture.defer = true;
  });
  const dialog = await openExport(page);
  await dialog
    .getByRole("button", { name: "Save transcript…", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("31 messages");
  await expect(
    dialog.getByRole("button", { name: "Prepare copy", exact: true }),
  ).toBeDisabled();
  await dialog
    .getByRole("button", { name: "Cancel export", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Export cancelled. Nothing was saved or copied.",
  );
  await page.evaluate(() => {
    const fixture = (window as any).transcriptFixture;
    fixture.defer = false;
    fixture.error = "History access was revoked. Nothing was saved.";
  });
  await dialog
    .getByRole("button", { name: "Save transcript…", exact: true })
    .click();
  await expect(dialog.getByRole("alert")).toHaveText(
    "History access was revoked. Nothing was saved.",
  );
  await expect(
    dialog.getByText("Conversation saved", { exact: false }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).transcriptFixture.clipboard),
  ).toEqual([]);
});

test("cancelling Copy suppresses a delayed native copy reply after cancellation acknowledgement", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  await page.evaluate(() => {
    const fixture = (window as any).transcriptFixture;
    fixture.defer = true;
    fixture.deliverCopyAfterCancel = true;
  });
  const dialog = await openExport(page);
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("31 messages");
  await dialog
    .getByRole("button", { name: "Cancel export", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText("Cancelling export…");
  const requests = await page.evaluate(
    () => (window as any).transcriptFixture.requests,
  );
  expect(requests[1]).toMatchObject({
    action: "cancel",
    operationId: requests[0].operationId,
    targetId: requests[0].targetId,
  });
  // Native has already released the formatted Copy reply; its cancellation
  // registration no longer exists, so that reply still arrives with status Copy.
  await page.evaluate(() => {
    (window as any).transcriptFixture.finish?.();
  });
  await expect(dialog.getByRole("status")).toHaveText(
    "Export cancelled. Nothing was saved or copied.",
  );
  await expect(
    dialog.getByText("Transcript copied", { exact: false }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).transcriptFixture.clipboard),
  ).toEqual([]);
});

test("delayed preparation waits for a fresh explicit Copy click", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  await page.evaluate(() => {
    (window as any).transcriptFixture.defer = true;
  });
  const dialog = await openExport(page);
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("31 messages");
  expect(
    await page.evaluate(
      () => (window as any).transcriptFixture.clipboardAttempts,
    ),
  ).toBe(0);
  await page.evaluate(() => {
    (window as any).transcriptFixture.finish?.();
  });
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(
      () => (window as any).transcriptFixture.clipboardAttempts,
    ),
  ).toBe(0);
  await dialog
    .getByRole("button", { name: "Copy transcript", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Transcript copied (31 messages).",
  );
  expect(
    await page.evaluate(
      () => (window as any).transcriptFixture.clipboardAttempts,
    ),
  ).toBe(1);
});

test("format changes and closing discard prepared clipboard text", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  let dialog = await openExport(page);
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("combobox", { name: "Format", exact: true }).click();
  await page
    .getByRole("option", { name: "Plain text (.txt)", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toHaveCount(0);
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  dialog = await openExport(page);
  await expect(
    dialog.getByRole("button", { name: "Prepare copy", exact: true }),
  ).toBeEnabled();
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as any).transcriptFixture.clipboardAttempts,
    ),
  ).toBe(0);
});

test("changing conversations discards already prepared clipboard text", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  const dialog = await openExport(page);
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(
    dialog.getByRole("button", { name: "Copy transcript", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "New thread in Colossus", exact: true })
    .evaluate((button: HTMLButtonElement) => button.click());
  await expect(dialog).toHaveCount(0);
  expect(
    await page.evaluate(
      () => (window as any).transcriptFixture.clipboardAttempts,
    ),
  ).toBe(0);
});

test("changing conversations discards an old Copy result and cancels the native operation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  await page.evaluate(() => {
    (window as any).transcriptFixture.defer = true;
  });
  const dialog = await openExport(page);
  await dialog
    .getByRole("button", { name: "Prepare copy", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toContainText("31 messages");
  // Simulate a host-driven selection change while the modal has browser focus.
  await page
    .getByRole("button", { name: "New thread in Colossus", exact: true })
    .evaluate((button: HTMLButtonElement) => button.click());
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).transcriptFixture.requests.filter(
            (request: any) => request.action === "cancel",
          ).length,
      ),
    )
    .toBe(1);
  expect(
    await page.evaluate(() => (window as any).transcriptFixture.clipboard),
  ).toEqual([]);
});

test("unsupported runtimes explain the gap without a transcript request", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio&exportUnsupported=1");
  await installExportBridge(page);
  const dialog = await openExport(page);
  await expect(dialog.getByRole("status")).toContainText(
    "does not support full conversation export",
  );
  await expect(
    dialog.getByRole("button", { name: "Save transcript…", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() => (window as any).transcriptFixture.requests),
  ).toEqual([]);
});

test("export dialog keeps Large text, keyboard controls and compact layouts across palettes", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await installExportBridge(page);
  const dialog = await openExport(page);
  await page.evaluate(() => {
    document.documentElement.dataset.textSize = "large";
  });
  for (const [theme, palette] of [
    ["light", "colossus"],
    ["dark", "colossus"],
    ["dark", "neutral"],
    ["dark", "black"],
    ["dark", "hacker"],
    ["dark", "forced"],
  ]) {
    await page.evaluate(
      ({ theme, palette }) => {
        document.documentElement.dataset.theme = theme;
        document.documentElement.dataset.palette =
          palette === "forced" ? "colossus" : palette;
      },
      { theme: theme!, palette: palette! },
    );
    await page.emulateMedia({
      forcedColors: palette === "forced" ? "active" : "none",
    });
    const screenshot = testInfo.outputPath(
      `export-${theme}-${palette}-large.png`,
    );
    await dialog.screenshot({ path: screenshot });
    await testInfo.attach(`${theme}-${palette}`, {
      path: screenshot,
      contentType: "image/png",
    });
    expect(
      await dialog.evaluate(
        (element) => element.scrollWidth <= element.clientWidth,
      ),
    ).toBe(true);
  }
  await page.emulateMedia({ forcedColors: "none" });
  await page.evaluate(() => {
    document.documentElement.dataset.palette = "neutral";
  });
  await page.setViewportSize({ width: 390, height: 640 });
  expect(
    await dialog.evaluate((element) => {
      const rect = element.getBoundingClientRect();
      return (
        rect.left >= 0 &&
        rect.right <= innerWidth &&
        rect.top >= 0 &&
        rect.bottom <= innerHeight
      );
    }),
  ).toBe(true);
  const compact = testInfo.outputPath("export-neutral-compact-large.png");
  await dialog
    .getByRole("combobox", { name: "Format", exact: true })
    .scrollIntoViewIfNeeded();
  await dialog.screenshot({ path: compact });
  await testInfo.attach("compact-large", {
    path: compact,
    contentType: "image/png",
  });
  const accessibility = await new AxeBuilder({ page })
    .include(".conversation-export-dialog")
    .analyze();
  expect(accessibility.violations).toEqual([]);
  await dialog.getByRole("button", { name: "Close", exact: true }).focus();
  await page.keyboard.press("Shift+Tab");
  await expect(
    dialog.getByRole("combobox", { name: "Format", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: "Close", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Conversation actions", exact: true }),
  ).toBeFocused();
});
