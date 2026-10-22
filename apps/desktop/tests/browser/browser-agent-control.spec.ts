import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function mountAgentBrowser(page: Page) {
  await page.goto("/?fixture=operations-studio");
  // Compact headers keep browser access in Tools rather than a shortcut icon.
  const tools = page.getByRole("button", { name: "Open tools", exact: true });
  await tools.focus();
  await tools.press("Enter");
  const browser = page.getByRole("menuitemradio", { name: /^Browser/ });
  await expect(browser).toBeVisible();
  await expect(browser).toBeEnabled();
  await browser.press("Escape");
  await expect(tools).toBeFocused();
  await page.evaluate(async () => {
    const resource = (name: string) =>
      performance
        .getEntriesByType("resource")
        .find((entry) => new URL(entry.name).pathname.endsWith(name))!.name;
    const { default: React } = await import(resource("/react.js"));
    const { default: ReactDOM } = await import(
      resource("/react-dom_client.js")
    );
    const panePath = "/src/components/browser/BrowserPane.tsx";
    const { BrowserPane } = await import(panePath);
    document.getElementById("root")!.hidden = true;
    const container = document.createElement("div");
    container.dataset.testid = "agent-browser";
    container.dataset.commands = "[]";
    container.style.cssText = "width: min(620px, 100vw); height: 600px";
    document.body.append(container);
    const viewport = async (request: unknown) => {
      container.dataset.viewport = JSON.stringify(request);
    };
    function Harness() {
      const [control, setControl] = React.useState("agent");
      const [failed, setFailed] = React.useState(false);
      const [completedSnapshot, setCompletedSnapshot] = React.useState(false);
      return React.createElement(
        React.Fragment,
        null,
        React.createElement(
          "div",
          null,
          ...[
            "agent",
            "paused",
            "human",
            "completed",
            "completed-snapshot",
            "expired",
            "unavailable",
          ].map((next) =>
            React.createElement(
              "button",
              {
                key: next,
                onClick: () => {
                  setFailed(next === "unavailable" || next === "expired");
                  setCompletedSnapshot(next === "completed-snapshot");
                  setControl(
                    next === "unavailable"
                      ? "agent"
                      : next === "completed-snapshot"
                        ? "completed"
                        : next,
                  );
                },
              },
              next,
            ),
          ),
        ),
        React.createElement(BrowserPane, {
          controller: {
            snapshot: {
              available: true,
              generation: 1,
              selectedTabId: "agent-tab",
              engine: {
                kind: "owned_chromium",
                ready: true,
                preview: false,
                message: null,
                agentControlAvailable: true,
              },
              tabs: [
                {
                  id: "agent-tab",
                  title: "Agent's page",
                  url: "https://example.com/",
                  canGoBack: false,
                  canGoForward: false,
                  loading: false,
                  error: failed
                    ? control === "expired"
                      ? "This browser session expired. Continue in a new session to browse again."
                      : "This browser session ended."
                    : null,
                  notice: null,
                  popupUrl: null,
                  control,
                  completedSnapshot,
                },
              ],
            },
            error: "",
            busy: false,
            loading: false,
            fixture: true,
            command: async (action: unknown) => {
              const commands = JSON.parse(container.dataset.commands!);
              commands.push(action);
              container.dataset.commands = JSON.stringify(commands);
            },
            viewport,
          },
          expanded: false,
          onExpand: () => {},
          onClose: () => {},
        }),
      );
    }
    ReactDOM.createRoot(container).render(React.createElement(Harness));
  });
  return page.getByTestId("agent-browser");
}

test("agent control stays visible while idle and stops claiming paused or ended sessions", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const harness = await mountAgentBrowser(page);
  const indicator = harness.getByRole("status").filter({
    hasText: /^Agent controlling$/,
  });
  await expect(indicator).toBeVisible();
  // A stationary pointer must not erase the session's control indicator.
  await page.waitForTimeout(2200);
  await expect(indicator).toBeVisible();
  const address = harness.getByRole("textbox", { name: "Web address" });
  await page.keyboard.press("Control+l");
  await expect(address).toBeFocused();
  await expect(address).toHaveAttribute("readonly");

  const bounds = await harness
    .getByLabel("Web page", { exact: true })
    .boundingBox();
  const frame = await harness.locator(".browser-viewport").boundingBox();
  const request = JSON.parse((await harness.getAttribute("data-viewport"))!);
  expect(request.rect).toEqual({
    x: bounds!.x,
    y: bounds!.y,
    width: bounds!.width,
    height: bounds!.height,
  });
  expect(bounds!.x - frame!.x).toBe(3);
  expect(bounds!.y - frame!.y).toBe(3);
  expect(frame!.width - bounds!.width).toBe(6);
  expect(frame!.height - bounds!.height).toBe(6);

  await harness.getByRole("button", { name: "paused", exact: true }).click();
  await expect(indicator).toHaveCount(0);
  await expect(harness.locator(".is-agent-controlled")).toHaveCount(0);
  await expect(harness.locator(".browser-footer")).toContainText("Paused");
  await harness.getByRole("button", { name: "human", exact: true }).click();
  await expect(indicator).toHaveCount(0);
  await expect(address).not.toHaveAttribute("readonly");
  await harness.getByRole("button", { name: "agent", exact: true }).click();
  await expect(indicator).toBeVisible();
  await harness
    .getByRole("button", { name: "unavailable", exact: true })
    .click();
  await expect(indicator).toHaveCount(0);
  await expect(
    harness.getByRole("heading", { name: "Agent browser unavailable" }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

test("expired browser preserves its address and continues only in a fresh session", async ({
  page,
}) => {
  const harness = await mountAgentBrowser(page);
  await harness.getByRole("button", { name: "expired", exact: true }).click();
  await expect(
    harness.getByRole("heading", { name: "Browser session expired" }),
  ).toBeVisible();
  await expect(
    harness.getByRole("textbox", { name: "Web address" }),
  ).toHaveValue("https://example.com/");
  await expect(
    harness.getByRole("textbox", { name: "Web address" }),
  ).toHaveAttribute("readonly");
  await expect(
    harness.getByRole("button", { name: "Reload page", exact: true }),
  ).toBeDisabled();
  await expect(harness).toHaveAttribute("data-commands", "[]");
  await expect
    .poll(
      async () =>
        JSON.parse((await harness.getAttribute("data-viewport"))!).rect,
    )
    .toBeNull();
  await harness
    .getByRole("button", { name: "Continue browsing", exact: true })
    .click();
  expect(JSON.parse((await harness.getAttribute("data-commands"))!)).toEqual([
    { type: "new", url: "https://example.com/" },
  ]);
  await expect(
    harness.getByRole("heading", { name: "Browser session expired" }),
  ).toBeVisible();
});

test("completed browser preserves a read-only last view and continues only in a fresh tab", async ({
  page,
}) => {
  const harness = await mountAgentBrowser(page);
  await expect(harness.locator(".browser-agent-status")).toBeVisible();
  await harness.getByRole("button", { name: "completed", exact: true }).click();
  await expect(harness.locator(".browser-completed-status")).toContainText(
    "Agent finished",
  );
  await expect(harness.locator(".browser-agent-status")).toHaveCount(0);
  await expect(harness.locator(".is-agent-controlled")).toHaveCount(0);
  await expect(
    harness.getByRole("heading", { name: "Last view unavailable" }),
  ).toBeVisible();
  await expect(
    harness.getByRole("textbox", { name: "Web address" }),
  ).toHaveAttribute("readonly");
  for (const name of ["Go back", "Go forward", "Reload page"]) {
    await expect(
      harness.getByRole("button", { name, exact: true }),
    ).toBeDisabled();
  }
  await expect(
    harness.getByRole("button", {
      name: "Close tab: Agent's page",
      exact: true,
    }),
  ).toBeEnabled();
  await expect
    .poll(
      async () =>
        JSON.parse((await harness.getAttribute("data-viewport"))!).rect,
    )
    .toBeNull();
  // Completion itself must never navigate or change the agent's authority.
  await expect(harness).toHaveAttribute("data-commands", "[]");
  await harness
    .getByRole("button", { name: "Continue browsing", exact: true })
    .click();
  expect(JSON.parse((await harness.getAttribute("data-commands"))!)).toEqual([
    { type: "new", url: "https://example.com/" },
  ]);
  await expect(harness.locator(".browser-completed-status")).toBeVisible();

  await harness
    .getByRole("button", { name: "completed-snapshot", exact: true })
    .click();
  await expect(harness.locator(".browser-completed-status")).toContainText(
    "Agent finished · Last view",
  );
  await expect(
    harness.getByRole("heading", { name: "Last view unavailable" }),
  ).toHaveCount(0);
  await expect
    .poll(
      async () =>
        JSON.parse((await harness.getAttribute("data-viewport"))!).rect,
    )
    .not.toBeNull();
  await expect(harness.locator(".is-agent-controlled")).toHaveCount(0);
});

for (const [colorTheme, darkPalette, textSize] of [
  ["dark", "colossus", "compact"],
  ["light", "colossus", "comfortable"],
  ["dark", "neutral", "large"],
  ["dark", "hacker", "large"],
] as const) {
  test(`agent frame remains readable in ${colorTheme} ${darkPalette} with ${textSize} text`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 380, height: 750 });
    await page.emulateMedia({ reducedMotion: "reduce", contrast: "more" });
    await page.addInitScript(
      ({ colorTheme, darkPalette, textSize }) => {
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({ colorTheme, darkPalette, textSize }),
        );
      },
      { colorTheme, darkPalette, textSize },
    );
    const harness = await mountAgentBrowser(page);
    await expect(
      harness.getByText("Agent controlling", { exact: true }),
    ).toBeInViewport();
    await expect(
      harness.getByRole("textbox", { name: "Web address" }),
    ).toBeInViewport();
    const frame = harness.locator(".is-agent-controlled");
    await expect(frame).toHaveCSS("border-top-width", "3px");
    const report = await new AxeBuilder({ page })
      .include('[data-testid="agent-browser"] .browser-pane')
      .analyze();
    expect(report.violations).toEqual([]);
    await harness.locator(".browser-pane").screenshot({
      path: `output/playwright/browser-agent-${colorTheme}-${darkPalette}.png`,
    });
    await page.emulateMedia({ forcedColors: "active" });
    await expect(frame).toHaveCSS("border-top-style", "solid");
    await expect(
      harness.getByText("Agent controlling", { exact: true }),
    ).toBeVisible();
    await page.emulateMedia({ forcedColors: "none" });
    await harness
      .getByRole("button", { name: "completed-snapshot", exact: true })
      .click();
    const completed = harness.locator(".browser-completed-status");
    await expect(completed).toBeInViewport();
    const continueBrowsing = completed.getByRole("button", {
      name: "Continue browsing",
    });
    await continueBrowsing.focus();
    await expect(continueBrowsing).toBeFocused();
    expect(
      (
        await new AxeBuilder({ page })
          .include('[data-testid="agent-browser"] .browser-pane')
          .analyze()
      ).violations,
    ).toEqual([]);
    await harness.locator(".browser-pane").screenshot({
      path: `output/playwright/browser-completed-${colorTheme}-${darkPalette}.png`,
    });
  });
}
