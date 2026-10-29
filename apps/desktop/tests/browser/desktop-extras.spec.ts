import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import {
  emitAppshot,
  installDesktopExtrasFixture,
} from "./support/desktop-extras";

test.beforeEach(async ({ page }) => {
  await installDesktopExtrasFixture(page);
});

test("App Shots preview, discard and explicit attach preserve the draft and never send", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Preserve this current chat draft.");
  const before = await page.locator('article[data-role="user"]').count();
  await emitAppshot(page, "capture-discard");
  const preview = page.getByRole("dialog", { name: "App Shot", exact: true });
  await expect(preview).toBeVisible();
  await preview.getByText("View captured text", { exact: true }).click();
  await expect(preview.locator("pre")).toContainText("cargo check --locked");
  await expect(prompt).toHaveValue("Preserve this current chat draft.");
  expect(
    (await new AxeBuilder({ page }).include(".appshot-preview").analyze())
      .violations,
  ).toEqual([]);
  await preview.screenshot({ path: testInfo.outputPath("appshot-review.png") });
  await preview.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(preview).toHaveCount(0);
  await expect(prompt).toHaveValue("Preserve this current chat draft.");
  await emitAppshot(page, "capture-attach");
  await expect(preview).toBeVisible();
  await preview
    .getByRole("checkbox", { name: "Include captured text" })
    .uncheck();
  await preview
    .getByRole("button", { name: "Attach to current draft", exact: true })
    .click();
  await expect(preview).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Remove owned-capture.png", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Remove owned-capture.txt", exact: true }),
  ).toHaveCount(0);
  await expect(prompt).toHaveValue("Preserve this current chat draft.");
  expect(await page.locator('article[data-role="user"]').count()).toBe(before);
  const actions = await page.evaluate(() =>
    (
      window as unknown as {
        desktopExtrasFixture: {
          calls: { command: string; args: Record<string, unknown> }[];
        };
      }
    ).desktopExtrasFixture.calls.filter(
      (call) =>
        call.command === "attach_appshot" || call.command === "discard_appshot",
    ),
  );
  expect(
    actions.some(
      (call) =>
        call.command === "discard_appshot" &&
        call.args.captureId === "capture-discard",
    ),
  ).toBe(true);
  expect(actions.filter((call) => call.command === "attach_appshot")).toEqual([
    {
      command: "attach_appshot",
      args: {
        captureId: "capture-attach",
        targetId: "fixture-managed-local",
        includeText: false,
      },
    },
  ]);
});

test("a capture crossing app context is discarded before it can become an attachment", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Keep the old workspace draft");
  await emitAppshot(page, "capture-context", false);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.evaluate(() =>
    (
      window as unknown as {
        desktopExtrasFixture: { emit(event: string, id: string): void };
      }
    ).desktopExtrasFixture.emit("appshot-ready", "capture-context"),
  );
  await expect(
    page.getByRole("dialog", { name: "App Shot", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    (
      window as unknown as {
        desktopExtrasFixture: { emit(event: string, value: unknown): void };
      }
    ).desktopExtrasFixture.emit("appshot-error", {
      captureId: "capture-context",
      message: "Late capture failure from the retired draft",
    }),
  );
  await expect(
    page.getByText("Late capture failure from the retired draft", {
      exact: true,
    }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await expect(prompt).toHaveValue("Keep the old workspace draft");
  const calls = await page.evaluate(() =>
    (
      window as unknown as {
        desktopExtrasFixture: { calls: { command: string }[] };
      }
    ).desktopExtrasFixture.calls.map((call) => call.command),
  );
  expect(calls).toContain("discard_appshot");
  expect(calls).not.toContain("attach_appshot");
});

for (const [theme, palette] of [
  ["light", "colossus"],
  ["dark", "neutral"],
  ["dark", "hacker"],
] as const) {
  test(`Composer and companion settings in ${theme} ${palette} remain usable at Large text`, async ({
    page,
  }, testInfo) => {
    await page.addInitScript(
      ({ theme, palette }) =>
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({
            colorTheme: theme,
            darkPalette: palette,
            textSize: "large",
          }),
        ),
      { theme, palette },
    );
    await page.setViewportSize({ width: 700, height: 990 });
    await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
    await page
      .getByRole("button", { name: "Open work navigation", exact: true })
      .click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Global", exact: true }).click();
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Composer & companion", exact: true })
      .click();
    const features = page.locator(".desktop-features");
    await expect(
      features.getByRole("switch", { name: "App Shots", exact: false }),
    ).not.toBeChecked();
    await expect(
      features.getByRole("switch", { name: "Show mini hacker", exact: false }),
    ).not.toBeChecked();
    const mode = features.getByRole("combobox", {
      name: "Suggest the next prompt",
      exact: false,
    });
    await expect(mode).toContainText("Local");
    await mode.click();
    await page.getByRole("option", { name: "Off", exact: true }).click();
    await expect(mode).toContainText("Off");
    await mode.click();
    await page
      .getByRole("option", { name: "Model · extra token usage", exact: true })
      .click();
    await expect(mode).toContainText("Model");
    await features
      .getByRole("switch", { name: "Show mini hacker", exact: false })
      .check();
    const size = features.getByRole("combobox", {
      name: "Companion size",
      exact: false,
    });
    await size.click();
    await page
      .getByRole("option", { name: "Comfortable", exact: true })
      .click();
    await expect(size).toContainText("Comfortable");
    await features
      .getByRole("switch", { name: "Animate companion", exact: false })
      .uncheck();
    await features
      .getByRole("heading", { name: "App Shots & companion", exact: true })
      .scrollIntoViewIfNeeded();
    await page.screenshot({
      path: testInfo.outputPath("desktop-extras-settings-viewport.png"),
    });
    await features.screenshot({
      path: testInfo.outputPath("desktop-extras-settings.png"),
    });
    expect(
      (
        await new AxeBuilder({ page })
          .include(".managed-settings-shell")
          .analyze()
      ).violations,
    ).toEqual([]);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}

test("mini hacker statuses, comfortable size, keyboard controls and reduced motion", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 156, height: 188 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?surface=developer-pet");
  const pet = page.getByRole("main", { name: "Colossus developer companion" });
  await expect(pet.getByRole("status")).toHaveText("Ready to build");
  for (const [status, label] of [
    ["working", "Working"],
    ["attention", "Your input is needed"],
    ["complete", "Work complete"],
  ]) {
    await page.evaluate(
      (status) =>
        ((
          window as unknown as {
            desktopExtrasFixture: { pet: { status: string } };
          }
        ).desktopExtrasFixture.pet.status = status),
      status,
    );
    await expect(pet.getByRole("status")).toHaveText(label!);
    await pet.screenshot({ path: testInfo.outputPath(`pet-${status}.png`) });
  }
  expect(
    await pet
      .locator(".pet-eyes")
      .evaluate((node) => getComputedStyle(node).animationName),
  ).toBe("none");
  await page.evaluate(() => {
    const fixture = (
      window as unknown as {
        desktopExtrasFixture: {
          pet: { size: string };
          emit(event: string, value: unknown): void;
        };
      }
    ).desktopExtrasFixture;
    fixture.pet.size = "large";
    fixture.emit("pet-settings-changed", null);
  });
  await expect(pet).toHaveClass(/pet-size-large/);
  const character = pet.locator(".pet-character");
  await character.dispatchEvent("pointerdown", { button: 2, isPrimary: true });
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: false });
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await character.dispatchEvent("pointerup", { button: 0, isPrimary: true });
  await pet.getByRole("button", { name: "Open Colossus" }).focus();
  await page.keyboard.press("Enter");
  await pet.getByRole("button", { name: "Hide companion" }).focus();
  await page.keyboard.press("Enter");
  const calls = await page.evaluate(() =>
    (
      window as unknown as {
        desktopExtrasFixture: {
          calls: { command: string; args: Record<string, unknown> }[];
        };
      }
    ).desktopExtrasFixture.calls.filter(
      (call) => call.command === "pet_action",
    ),
  );
  expect(calls.map((call) => call.args.action)).toEqual([
    "drag",
    "save_position",
    "open",
    "hide",
  ]);
  expect(calls.find((call) => call.args.action === "drag")?.args).toEqual({
    action: "drag",
  });
  expect(
    await page.evaluate(() =>
      (
        window as unknown as {
          desktopExtrasFixture: { calls: { command: string }[] };
        }
      ).desktopExtrasFixture.calls.some((call) =>
        call.command.startsWith("plugin:window|"),
      ),
    ),
  ).toBe(false);
  expect(
    await page.evaluate(
      () =>
        document.documentElement.scrollWidth <= innerWidth &&
        document.documentElement.scrollHeight <= innerHeight,
    ),
  ).toBe(true);
  await pet.screenshot({ path: testInfo.outputPath("pet-comfortable.png") });
});

test("a second hotkey retires the earlier preview and cannot attach the wrong capture", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  await page
    .getByRole("textbox", { name: "Prompt", exact: true })
    .fill("Keep this draft.");
  await emitAppshot(page, "capture-first");
  const preview = page.getByRole("dialog", { name: "App Shot", exact: true });
  await expect(preview).toBeVisible();
  await emitAppshot(page, "capture-second", false);
  await expect(preview).toHaveCount(0);
  await page.evaluate(() =>
    (
      window as unknown as {
        desktopExtrasFixture: { emit(event: string, id: string): void };
      }
    ).desktopExtrasFixture.emit("appshot-ready", "capture-second"),
  );
  await expect(preview).toBeVisible();
  await preview
    .getByRole("button", { name: "Attach to current draft", exact: true })
    .click();
  const calls = await page.evaluate(
    () =>
      (
        window as unknown as {
          desktopExtrasFixture: {
            calls: { command: string; args: Record<string, unknown> }[];
          };
        }
      ).desktopExtrasFixture.calls,
  );
  expect(
    calls.some(
      (call) =>
        call.command === "discard_appshot" &&
        call.args.captureId === "capture-first",
    ),
  ).toBe(true);
  expect(
    calls
      .filter((call) => call.command === "attach_appshot")
      .map((call) => call.args.captureId),
  ).toEqual(["capture-second"]);
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue("Keep this draft.");
});

test("native Hide refreshes companion settings and unrelated updates preserve hidden state", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Composer & companion", exact: true })
    .click();
  const pet = page.getByRole("switch", {
    name: "Show mini hacker",
    exact: false,
  });
  await pet.check();
  await page.evaluate(async () => {
    const api = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            args: Record<string, unknown>,
          ): Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    await api.invoke("pet_action", { action: "hide" });
  });
  await expect(pet).not.toBeChecked();
  await page
    .getByRole("switch", { name: "Animate companion", exact: false })
    .uncheck();
  await expect(pet).not.toBeChecked();
  const saved = await page.evaluate(
    () =>
      (
        window as unknown as {
          desktopExtrasFixture: {
            preferences: { petEnabled: boolean };
            calls: { command: string; args: Record<string, unknown> }[];
          };
        }
      ).desktopExtrasFixture,
  );
  expect(saved.preferences.petEnabled).toBe(false);
});

test("a stale settings revision cannot reopen a companion hidden by the native window", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page
    .getByRole("navigation", { name: "Settings sections" })
    .getByRole("button", { name: "Composer & companion", exact: true })
    .click();
  await page
    .getByRole("switch", { name: "Show mini hacker", exact: false })
    .check();
  await page.evaluate(() => {
    const fixture = (
      window as unknown as {
        desktopExtrasFixture: {
          revision: number;
          preferences: { petEnabled: boolean };
        };
      }
    ).desktopExtrasFixture;
    fixture.revision += 1;
    fixture.preferences.petEnabled = false;
  });
  await page
    .getByRole("switch", { name: "Animate companion", exact: false })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "Desktop settings changed",
  );
  expect(
    await page.evaluate(
      () =>
        (
          window as unknown as {
            desktopExtrasFixture: { preferences: { petEnabled: boolean } };
          }
        ).desktopExtrasFixture.preferences.petEnabled,
    ),
  ).toBe(false);
});

test("a long unbroken captured window title stays within its review dialog", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 700, height: 990 });
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.desktop.appearance.v1",
      JSON.stringify({
        colorTheme: "dark",
        darkPalette: "hacker",
        textSize: "large",
      }),
    ),
  );
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toBeVisible();
  await emitAppshot(page, "capture-long-title", false);
  await page.evaluate(() => {
    const fixture = (
      window as unknown as {
        desktopExtrasFixture: {
          captures: Record<string, { windowTitle: string }>;
          emit(event: string, value: unknown): void;
        };
      }
    ).desktopExtrasFixture;
    fixture.captures["capture-long-title"]!.windowTitle =
      "OwnedFixture" + "x".repeat(390);
    fixture.emit("appshot-ready", "capture-long-title");
  });
  const preview = page.getByRole("dialog", { name: "App Shot", exact: true });
  await expect(preview).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("appshot-long-title-viewport.png"),
  });
  expect(
    await preview.evaluate(
      (dialog) => dialog.scrollWidth <= dialog.clientWidth,
    ),
  ).toBe(true);
  await preview.getByRole("button", { name: "Discard", exact: true }).click();
});

for (const [theme, palette] of [
  ["light", "colossus"],
  ["dark", "colossus"],
  ["dark", "neutral"],
  ["dark", "hacker"],
] as const) {
  for (const textSize of ["compact", "large"] as const) {
    test(`mini hacker ${theme} ${palette} ${textSize} keeps the native window bounds`, async ({
      page,
    }, testInfo) => {
      await page.addInitScript(
        ({ theme, palette, textSize }) =>
          localStorage.setItem(
            "colossus.desktop.appearance.v1",
            JSON.stringify({
              colorTheme: theme,
              darkPalette: palette,
              textSize,
            }),
          ),
        { theme, palette, textSize },
      );
      await page.setViewportSize({ width: 156, height: 188 });
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/?surface=developer-pet");
      const pet = page.getByRole("main", {
        name: "Colossus developer companion",
      });
      await expect(pet).toBeVisible();
      await page.evaluate(() => {
        const fixture = (
          window as unknown as {
            desktopExtrasFixture: {
              pet: { size: string; status: string };
              emit(event: string, value: unknown): void;
            };
          }
        ).desktopExtrasFixture;
        fixture.pet.size = "large";
        fixture.pet.status = "attention";
        fixture.emit("pet-settings-changed", null);
      });
      await expect(pet).toHaveClass(/pet-size-large/);
      await expect(pet.getByRole("status")).toHaveText("Your input is needed");
      await page.screenshot({
        path: testInfo.outputPath("pet-large-attention-viewport.png"),
      });
      expect(
        await page.evaluate(() => ({
          width: document.documentElement.scrollWidth,
          height: document.documentElement.scrollHeight,
          viewportWidth: innerWidth,
          viewportHeight: innerHeight,
        })),
      ).toEqual({
        width: 156,
        height: 188,
        viewportWidth: 156,
        viewportHeight: 188,
      });
      expect(
        await pet
          .locator(".pet-eyes")
          .evaluate((node) => getComputedStyle(node).animationName),
      ).toBe("none");
    });
  }
}

test("late Attach rejection from a replaced capture cannot mark the new same-chat preview", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Preserve the draft during capture replacement.");
  await page.evaluate(
    () =>
      ((
        window as unknown as { desktopExtrasFixture: { delayAttach: boolean } }
      ).desktopExtrasFixture.delayAttach = true),
  );
  await emitAppshot(page, "capture-late-attach");
  const preview = page.getByRole("dialog", { name: "App Shot", exact: true });
  await preview
    .getByRole("button", { name: "Attach to current draft", exact: true })
    .click();
  await expect(
    preview.getByRole("button", { name: "Attaching…", exact: true }),
  ).toBeDisabled();
  await emitAppshot(page, "capture-replacement", false);
  await page.evaluate(() => {
    const fixture = (
      window as unknown as {
        desktopExtrasFixture: {
          captures: Record<string, { windowTitle: string }>;
          emit(event: string, value: unknown): void;
        };
      }
    ).desktopExtrasFixture;
    fixture.captures["capture-replacement"]!.windowTitle =
      "Replacement Owned Fixture";
    fixture.emit("appshot-ready", "capture-replacement");
  });
  await expect(preview).toContainText("Replacement Owned Fixture");
  await page.evaluate(() => {
    const fixture = (
      window as unknown as {
        desktopExtrasFixture: {
          emit(event: string, value: unknown): void;
          rejectAttach: ((message: string) => void) | null;
        };
      }
    ).desktopExtrasFixture;
    fixture.emit("appshot-error", {
      captureId: "capture-late-attach",
      message: "Retired native capture error",
    });
    fixture.rejectAttach!("Late attachment error from retired capture");
  });
  await expect(preview).toContainText("Replacement Owned Fixture");
  await expect(preview.getByRole("alert")).toHaveCount(0);
  await expect(
    preview.getByRole("button", {
      name: "Attach to current draft",
      exact: true,
    }),
  ).toBeEnabled();
  await expect(prompt).toHaveValue(
    "Preserve the draft during capture replacement.",
  );
  await preview.getByRole("button", { name: "Discard", exact: true }).click();
});

for (const delayed of ["save", "read", "save-error"] as const) {
  test(`older settings responses cannot overwrite a newer native Hide: ${delayed}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/?fixture=operations-studio&fixtureAttachments=1");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Global", exact: true }).click();
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Composer & companion", exact: true })
      .click();
    const pet = page.getByRole("switch", {
      name: "Show mini hacker",
      exact: false,
    });
    const animate = page.getByRole("switch", {
      name: "Animate companion",
      exact: false,
    });
    const shortcut = page.getByRole("textbox", {
      name: "Capture shortcut",
      exact: false,
    });
    await pet.check();
    await expect(pet).toBeEnabled();
    await page.evaluate((delayed) => {
      const host = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            args?: Record<string, unknown>,
          ): Promise<unknown>;
        };
        desktopExtrasFixture: {
          releaseDesktopResponse?: () => void;
          emit(event: string, payload: unknown): void;
        };
      };
      const original = host.__TAURI_INTERNALS__.invoke.bind(
        host.__TAURI_INTERNALS__,
      );
      let intercepted = false;
      host.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (
          !intercepted &&
          command ===
            (delayed === "read"
              ? "desktop_features_context"
              : "save_desktop_features")
        ) {
          intercepted = true;
          const snapshot = await original(command, args);
          return new Promise((resolve, reject) => {
            host.desktopExtrasFixture.releaseDesktopResponse = () => {
              host.__TAURI_INTERNALS__.invoke = original;
              if (delayed === "save-error")
                reject({ message: "Stale save failure" });
              else resolve(snapshot);
            };
          });
        }
        return original(command, args);
      };
      if (delayed === "read")
        host.desktopExtrasFixture.emit("desktop-features-changed", null);
    }, delayed);
    if (delayed !== "read") await animate.click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            typeof (
              window as unknown as {
                desktopExtrasFixture: { releaseDesktopResponse?: () => void };
              }
            ).desktopExtrasFixture.releaseDesktopResponse,
        ),
      )
      .toBe("function");
    await page.evaluate(async () => {
      const host = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            args: Record<string, unknown>,
          ): Promise<unknown>;
        };
        desktopExtrasFixture: { preferences: { appshotShortcut: string } };
      };
      host.desktopExtrasFixture.preferences.appshotShortcut = "Control+Alt+H";
      await host.__TAURI_INTERNALS__.invoke("pet_action", { action: "hide" });
    });
    await expect(pet).not.toBeChecked();
    await expect(shortcut).toHaveValue("Control+Alt+H");
    await page.evaluate(() =>
      (
        window as unknown as {
          desktopExtrasFixture: { releaseDesktopResponse(): void };
        }
      ).desktopExtrasFixture.releaseDesktopResponse(),
    );
    await expect(animate).toBeEnabled();
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    await expect(pet).not.toBeChecked();
    await expect(shortcut).toHaveValue("Control+Alt+H");
    await expect(
      page.getByText("Stale save failure", { exact: true }),
    ).toHaveCount(0);
  });
}

test("companion drag failures remain visible without generic window commands", async ({
  page,
}) => {
  await page.setViewportSize({ width: 156, height: 188 });
  await page.goto("/?surface=developer-pet");
  const pet = page.getByRole("main", { name: "Colossus developer companion" });
  await expect(pet.getByRole("status")).toHaveText("Ready to build");
  await page.evaluate(() => {
    const api = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke(
            command: string,
            args?: Record<string, unknown>,
          ): Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    const original = api.invoke.bind(api);
    api.invoke = async (command, args) => {
      if (command === "pet_action" && args?.action === "drag") {
        await original(command, args);
        throw { message: "The companion could not move." };
      }
      return original(command, args);
    };
  });
  await pet
    .locator(".pet-character")
    .dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await expect(pet.getByRole("alert")).toHaveText(
    "The companion could not move.",
  );
  const calls = await page.evaluate(
    () =>
      (
        window as unknown as {
          desktopExtrasFixture: {
            calls: { command: string; args: Record<string, unknown> }[];
          };
        }
      ).desktopExtrasFixture.calls,
  );
  expect(calls.filter((call) => call.command === "pet_action")).toEqual([
    { command: "pet_action", args: { action: "drag" } },
  ]);
  expect(calls.some((call) => call.command.startsWith("plugin:window|"))).toBe(
    false,
  );
});

test("companion Large errors remain readable, retryable and keyboard accessible inside the native window", async ({
  page,
}, testInfo) => {
  await page.addInitScript(() =>
    localStorage.setItem(
      "colossus.desktop.appearance.v1",
      JSON.stringify({
        colorTheme: "dark",
        darkPalette: "hacker",
        textSize: "large",
      }),
    ),
  );
  await page.setViewportSize({ width: 156, height: 188 });
  await page.goto("/?surface=developer-pet");
  const pet = page.getByRole("main", { name: "Colossus developer companion" });
  await expect(pet.getByRole("status")).toHaveText("Ready to build");
  await page.evaluate(() => {
    const host = window as unknown as {
      desktopExtrasFixture: { pet: { size: string } };
      __TAURI_INTERNALS__: {
        invoke(
          command: string,
          args?: Record<string, unknown>,
        ): Promise<unknown>;
      };
      petErrorFixture: {
        message: string | null;
        delayNextDrag: boolean;
        rejectDrag: ((error: unknown) => void) | null;
      };
    };
    host.desktopExtrasFixture.pet.size = "large";
    host.petErrorFixture = {
      message: "The companion could not move.",
      delayNextDrag: false,
      rejectDrag: null,
    };
    const original = host.__TAURI_INTERNALS__.invoke.bind(
      host.__TAURI_INTERNALS__,
    );
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const result = await original(command, args);
      if (command === "pet_action" && args?.action === "drag") {
        if (host.petErrorFixture.delayNextDrag) {
          host.petErrorFixture.delayNextDrag = false;
          return new Promise((_, reject) => {
            host.petErrorFixture.rejectDrag = reject;
          });
        }
        if (host.petErrorFixture.message)
          throw { message: host.petErrorFixture.message };
      }
      return result;
    };
  });
  await expect(pet).toHaveClass(/pet-size-large/);
  const character = pet.locator(".pet-character");
  const alert = pet.getByRole("alert");
  const open = pet.getByRole("button", { name: "Open Colossus" });
  const hide = pet.getByRole("button", { name: "Hide companion" });
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await expect(alert).toHaveText("The companion could not move.");
  await expect(pet.getByRole("status")).toHaveCount(0);
  await character.dispatchEvent("pointerup", { button: 0, isPrimary: true });
  await expect(alert).toHaveText("The companion could not move.");
  for (const element of [pet, alert, open, hide]) {
    const bounds = await element.boundingBox();
    expect(bounds).not.toBeNull();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(156);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(188);
  }
  await testInfo.attach("pet-error-bounds", {
    body: JSON.stringify({
      alert: await alert.boundingBox(),
      open: await open.boundingBox(),
      hide: await hide.boundingBox(),
      viewport: { width: 156, height: 188 },
    }),
    contentType: "application/json",
  });
  await page.screenshot({
    path: testInfo.outputPath("pet-large-native-error-viewport.png"),
  });
  const longMessage = `${"Native window movement failed. ".repeat(8)}${"detail".repeat(20)} End of native diagnostic.`;
  await page.evaluate((message) => {
    (
      window as unknown as { petErrorFixture: { message: string } }
    ).petErrorFixture.message = message;
  }, longMessage);
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await expect(alert).toHaveText(longMessage);
  expect(
    await alert.evaluate((node) => node.scrollHeight > node.clientHeight),
  ).toBe(true);
  await alert.focus();
  await expect(alert).toBeFocused();
  await page.keyboard.press("End");
  await expect
    .poll(() => alert.evaluate((node) => node.scrollTop))
    .toBeGreaterThan(0);
  await page.screenshot({
    path: testInfo.outputPath("pet-large-error-scroll-viewport.png"),
  });
  await page.keyboard.press("Tab");
  await expect(open).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(hide).toBeFocused();
  expect(
    await page.evaluate(() => ({
      width: document.documentElement.scrollWidth,
      height: document.documentElement.scrollHeight,
    })),
  ).toEqual({ width: 156, height: 188 });
  await page.evaluate(() => {
    (
      window as unknown as { petErrorFixture: { message: null } }
    ).petErrorFixture.message = null;
  });
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await expect(alert).toHaveCount(0);
  await expect(pet.getByRole("status")).toHaveText("Ready to build");
  await page.evaluate(() => {
    (
      window as unknown as { petErrorFixture: { delayNextDrag: boolean } }
    ).petErrorFixture.delayNextDrag = true;
  });
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { petErrorFixture: { rejectDrag: unknown } })
            .petErrorFixture.rejectDrag !== null,
      ),
    )
    .toBe(true);
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await page.evaluate(() => {
    (
      window as unknown as {
        petErrorFixture: { rejectDrag: (failure: unknown) => void };
      }
    ).petErrorFixture.rejectDrag({ message: "Retired drag failure" });
  });
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await expect(alert).toHaveCount(0);
  await expect(pet.getByRole("status")).toHaveText("Ready to build");
  await page.evaluate(() => {
    (
      window as unknown as { petErrorFixture: { message: string } }
    ).petErrorFixture.message = "A retryable drag failure";
  });
  await character.dispatchEvent("pointerdown", { button: 0, isPrimary: true });
  await expect(alert).toHaveText("A retryable drag failure");
  await open.focus();
  await page.keyboard.press("Enter");
  await expect(alert).toHaveCount(0);
  await expect(pet.getByRole("status")).toHaveText("Ready to build");
});
