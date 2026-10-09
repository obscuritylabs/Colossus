// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ThreadAgentInboxes } from "./ThreadAgentInboxes";
import type { Task } from "./api";

const task: Task = {
  task_id: "task-1",
  project_id: "project-1",
  node_id: "node-1",
  subject: "user-1",
  request: {
    input: [{ text: "Review messaging" }],
    role: "primary",
    mode: "execute",
  },
  run_id: "run-1",
  snapshot: null,
  last_sequence: 0,
  revision: 1,
};
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function expand() {
  await act(async () => {
    const details = container.querySelector("details")!;
    details.open = true;
    details.dispatchEvent(new Event("toggle"));
  });
}

it("lazy inbox inspection queues the exact cloud payload with CSRF and renders released peer text", async () => {
  const reads: Record<string, unknown>[] = [];
  const fetch = vi.fn(async (url: string, options: RequestInit) => {
    expect(url).toBe("/api/projects/project-1/tasks/task-1/inboxes");
    expect(options.method).toBe("POST");
    expect(options.credentials).toBe("same-origin");
    expect(options.headers).toMatchObject({ "X-Colossus-CSRF": "1" });
    const request = JSON.parse(options.body as string) as Record<
      string,
      unknown
    >;
    expect(Object.keys(request).sort()).toEqual([
      "after_sequence",
      "participant_id",
      "request_id",
    ]);
    reads.push(request);
    return new Response(
      JSON.stringify({
        command: {
          command_id: `read-${reads.length}`,
          reply: {
            kind: "inboxes",
            participants: [
              {
                id: "participant-1",
                root_run_id: "run-1",
                run_id: "run-1",
                session_id: "session-1",
                parent_id: null,
                subagent_id: null,
                generation: 1,
                open: false,
                closed_reason: "completed",
                pending_messages: 0,
                pending_bytes: 0,
                created_at: "2026-10-09T12:00:00Z",
              },
            ],
            page:
              request.participant_id === null
                ? null
                : {
                    messages: [
                      {
                        id: "message-1",
                        root_run_id: "run-1",
                        recipient_id: "participant-1",
                        sequence: 1,
                        sender: {
                          kind: "application",
                          application_id: "application-1",
                        },
                        text: "<script>peerInput()</script>",
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
                  },
          },
        },
      }),
    );
  });
  vi.stubGlobal("fetch", fetch);
  await act(() =>
    root.render(
      <ThreadAgentInboxes project="project-1" task={task} available />,
    ),
  );
  expect(fetch).not.toHaveBeenCalled();
  await expand();
  expect(reads).toHaveLength(2);
  expect(reads[0]?.participant_id).toBeNull();
  expect(reads[1]?.participant_id).toBe("participant-1");
  expect(container.querySelector("pre")?.textContent).toBe(
    "<script>peerInput()</script>",
  );
  expect(container.querySelector("script")).toBeNull();
  expect(container.textContent).toContain("Included in prepared turn 2");
});

it("shared source inspection does not borrow the source application's inbox permission", async () => {
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  await act(() =>
    root.render(
      <ThreadAgentInboxes
        project="project-1"
        task={{ ...task, source_read_only: true }}
        available
      />,
    ),
  );
  await expand();
  expect(fetch).not.toHaveBeenCalled();
  expect(container.textContent).toContain(
    "This connection does not provide inbox inspection",
  );
});
