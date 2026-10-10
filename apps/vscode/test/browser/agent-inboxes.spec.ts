import { expect, test } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";

test("the inspector reads host-bound inboxes and keeps peer markup inert", async ({
  page,
}, testInfo) => {
  await page.addInitScript(() => {
    const host = window as unknown as {
      acquireVsCodeApi: () => unknown;
      inboxActions: unknown[];
    };
    host.inboxActions = [];
    host.acquireVsCodeApi = () => ({
      postMessage(value: {
        type: string;
        requestId?: string;
        participantId?: string | null;
      }) {
        host.inboxActions.push(value);
        if (value.type !== "agentInbox") return;
        const participant = {
          id: "participant-1",
          root_run_id: "run-1",
          session_id: "session-1",
          run_id: "run-1",
          parent_id: null,
          subagent_id: null,
          generation: 1,
          open: false,
          closed_reason: "completed",
          pending_messages: 0,
          pending_bytes: 0,
          created_at: "2026-10-09T12:00:00Z",
        };
        const page =
          value.participantId === null
            ? null
            : {
                messages: [
                  {
                    id: "message-1",
                    root_run_id: "run-1",
                    recipient_id: "participant-1",
                    sender: {
                      kind: "application",
                      application_id: "application-1",
                    },
                    sequence: 1,
                    text: "<script>peerText()</script>",
                    reply_to: null,
                    accepted_at: "2026-10-09T12:00:00Z",
                    receipt: {
                      state: "included_in_turn",
                      run_id: "run-1",
                      turn: 2,
                      request_hash: "a".repeat(64),
                    },
                  },
                ],
                next_sequence: 1,
                has_more: false,
              };
        queueMicrotask(() =>
          window.postMessage(
            {
              type: "agentInbox",
              requestId: value.requestId,
              payload: { participants: [participant], page },
            },
            "*",
          ),
        );
      },
      getState: () => undefined,
      setState() {},
    });
  });
  await page.goto("/inspector");
  await page.evaluate(() =>
    window.postMessage(
      {
        type: "inspection",
        connected: true,
        loading: false,
        error: "",
        view: {
          agentInboxesAvailable: true,
          run: {
            id: "run-1",
            sessionId: "session-1",
            title: "Agent messaging",
            role: "primary",
            mode: "execute",
            status: "completed",
            createdAt: "2026-10-09T12:00:00Z",
            updatedAt: "2026-10-09T12:00:00Z",
            startedAt: "2026-10-09T12:00:00Z",
            finishedAt: "2026-10-09T12:00:00Z",
            sequence: "10",
            pendingInteractions: 0,
          },
          model: "offline",
          provider: "fixture",
          output: "",
          activities: [],
          activityState: "",
          activityHasMore: false,
          observedAt: "2026-10-09T12:00:00Z",
        },
      },
      "*",
    ),
  );
  await page
    .getByRole("button", { name: "Agent inboxes", exact: true })
    .click();
  const inbox = page.getByRole("region", { name: "Agent inboxes" });
  await expect(inbox.locator("article")).toHaveCount(1);
  await expect(inbox.locator("pre")).toHaveText("<script>peerText()</script>");
  await expect(inbox.locator("script, img, a")).toHaveCount(0);
  await expect(inbox).toContainText("Included in prepared turn 2");
  await inbox.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(inbox.locator("article")).toHaveCount(1);
  const actions = await page.evaluate(
    () =>
      (window as unknown as { inboxActions: Record<string, unknown>[] })
        .inboxActions,
  );
  const reads = actions.filter((action) => action.type === "agentInbox");
  expect(reads.length).toBeGreaterThanOrEqual(4);
  expect(
    reads.every(
      (action) =>
        Object.keys(action).sort().join(",") ===
        "afterSequence,participantId,requestId,type",
    ),
  ).toBe(true);
  expect(
    (await new AxeBuilder({ page }).include(".agent-inbox").analyze())
      .violations,
  ).toEqual([]);
  await inbox.screenshot({
    path: testInfo.outputPath("vscode-agent-inbox.png"),
  });
  await page.evaluate(() => document.body.classList.add("vscode-light"));
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(
    (await new AxeBuilder({ page }).include(".agent-inbox").analyze())
      .violations,
  ).toEqual([]);
});
