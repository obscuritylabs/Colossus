import { expect, test } from "@playwright/test";

test("browser activity reveals the selected run once and respects manual dismissal", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("button", { name: "Open browser", exact: true }),
  ).toBeVisible();
  await page.evaluate(async () => {
    const resource = (name: string) =>
      performance
        .getEntriesByType("resource")
        .find((entry) => new URL(entry.name).pathname.endsWith(name))!.name;
    const { default: React } = await import(resource("/react.js"));
    const { default: ReactDOM } = await import(
      resource("/react-dom_client.js")
    );
    const workPath = "/src/components/WorkSurface.tsx";
    const appearancePath = "/src/theme/AppearanceProvider.tsx";
    const fixturePath = "/src/dev/operations-studio-fixture.ts";
    const { WorkSurface } = await import(workPath);
    const { AppearanceProvider } = await import(appearancePath);
    const { buildOperationsStudioFixture } = await import(fixturePath);
    const fixture = buildOperationsStudioFixture();
    const original = fixture.views.get(fixture.activeRunId)!;
    document.getElementById("root")!.hidden = true;
    const container = document.createElement("div");
    container.dataset.testid = "browser-visibility";
    document.body.append(container);
    (
      window as unknown as { __TAURI_INTERNALS__: unknown }
    ).__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: { runId?: string }) => {
        if (command === "list_setup_packages") return [];
        if (command !== "browser_context")
          throw new Error("Fixture command unavailable");
        container.dataset.watchedRun = args.runId ?? "";
        return {
          available: true,
          generation: 1,
          tabs: [],
          selectedTabId: null,
        };
      },
    };
    const noop = () => undefined;
    function Harness() {
      const [activity, setActivity] = React.useState(0);
      const [runId, setRunId] = React.useState("run-A");
      const [completed, setCompleted] = React.useState(false);
      const view = {
        ...original,
        run: {
          ...original.run,
          runId,
          status: completed ? "completed" : "running",
        },
        pendingInteractions: [],
        updates: activity
          ? [
              {
                runId,
                sequence: activity,
                createdAt: original.run.createdAt,
                update: {
                  type: "tool_activity",
                  activity: {
                    callId: `call-${activity}`,
                    toolName: "browser.snapshot",
                    state: "started",
                    summary: "Inspecting browser",
                  },
                },
              },
            ]
          : [],
      };
      return React.createElement(
        React.Fragment,
        null,
        React.createElement(
          "button",
          { onClick: () => setActivity((value: number) => value + 1) },
          "Browser action",
        ),
        React.createElement(
          "button",
          {
            onClick: () => {
              setCompleted(true);
              setRunId("history");
            },
          },
          "Show completed history",
        ),
        React.createElement(
          "button",
          {
            onClick: () => {
              setCompleted(false);
              setRunId("run-B");
            },
          },
          "Next run",
        ),
        React.createElement(WorkSurface, {
          title: "Browser visibility",
          view,
          conversationViews: [view],
          connection: {
            state: "connected",
            message: "Connected",
            targetId: "workspace-A",
          },
          connecting: false,
          cancelling: false,
          runLoadError: "",
          actionError: null,
          participants: [],
          sessionMap: null,
          sessionMapLoading: false,
          sessionMapError: "",
          selectedParticipantId: null,
          delegateView: undefined,
          delegateInspection: null,
          delegateLoading: false,
          delegateError: "",
          artifacts: [],
          selectedSpaceName: "Workspace A",
          threadPinned: false,
          followRequestSequence: 0,
          composer: () => React.createElement("div"),
          filesPanel: null,
          filesAvailable: false,
          artifactsAvailable: false,
          asideView: undefined,
          asideConversationViews: [],
          asideHistory: [],
          asideBusy: false,
          asideError: null,
          asideReadOnly: false,
          planContinuationAvailable: false,
          planWorkflowAvailable: false,
          workNavigationOpen: false,
          onCloseWorkNavigation: noop,
          onBackToThreadDetails: noop,
          browserScope: "workspace-A",
        }),
      );
    }
    ReactDOM.createRoot(container).render(
      React.createElement(
        AppearanceProvider,
        null,
        React.createElement(Harness),
      ),
    );
  });
  const harness = page.getByTestId("browser-visibility");
  const browser = harness.getByRole("region", { name: "Browser", exact: true });
  await expect(browser).toHaveCount(0);
  await harness
    .getByRole("button", { name: "Browser action", exact: true })
    .click();
  await expect(browser).toBeVisible();
  await expect(harness).toHaveAttribute("data-watched-run", "run-A");
  await harness
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await harness
    .getByRole("button", { name: "Browser action", exact: true })
    .click();
  await expect(browser).toHaveCount(0);
  await harness.getByRole("button", { name: "Show completed history" }).click();
  await expect(browser).toHaveCount(0);
  await harness.getByRole("button", { name: "Next run" }).click();
  await expect(browser).toBeVisible();
  await expect(harness).toHaveAttribute("data-watched-run", "run-B");
  await page.screenshot({
    path: "output/playwright/browser-agent-visible.png",
  });
});
