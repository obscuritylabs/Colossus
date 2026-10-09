import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createWorkflowHost,
  RuntimeResourceFailure,
} from "./runtime-resources";
const caps = [
  "workflows.read",
  "workflows.register",
  "schedules.read",
  "schedules.create",
  "schedules.control",
  "schedules.delete",
  "workflow_runs.read",
  "workflow_runs.start",
  "schedules.calendar",
  "schedules.tasks",
  "workflow_runs.history",
];
const response = (value: unknown, connection = "connection-one") =>
  new Response(
    JSON.stringify({ kind: "result", connection_id: connection, value }),
    { status: 200 },
  );
afterEach(() => vi.unstubAllGlobals());
describe("runtime management host adapter", () => {
  it("intersects runtime scopes with project permissions and fences reviewed forms to their connection", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(response({ capabilities: caps }))
      .mockResolvedValueOnce(response({ items: [], next_cursor: null }));
    vi.stubGlobal("fetch", fetch);
    const host = createWorkflowHost("project-a", ["read"], () => ["default"]);
    const context = await host.workflowContext("node-a");
    expect(context.workflows_read).toBe(true);
    expect(context.workflows_register).toBe(false);
    expect(context.schedules_control).toBe(false);
    expect(context.workflow_runs_start).toBe(false);
    await host.listRegisteredWorkflows("node-a", context.selection_epoch, null);
    expect(fetch.mock.calls[1]![0]).toBe(
      "/api/projects/project-a/nodes/node-a/resources",
    );
    expect(JSON.parse(fetch.mock.calls[1]![1].body).connection_id).toBe(
      "connection-one",
    );
    expect(await host.taskModelProfiles("node-a")).toEqual(["default"]);
    await expect(
      host.listRegisteredWorkflows("other-node", context.selection_epoch, null),
    ).rejects.toThrow("Workspace changed");
    expect(fetch).toHaveBeenCalledTimes(2);
  });
  it("supports authorized management and invalidates earlier review epochs on refresh", async () => {
    const fetch = vi
      .fn()
      .mockImplementation(() =>
        Promise.resolve(response({ capabilities: caps })),
      );
    vi.stubGlobal("fetch", fetch);
    const host = createWorkflowHost(
      "project-a",
      ["read", "execute", "control"],
      () => [],
    );
    const first = await host.workflowContext("node-a");
    const second = await host.workflowContext("node-a");
    expect(second.schedules_create).toBe(true);
    expect(second.schedules_delete).toBe(true);
    expect(second.workflows_register).toBe(true);
    await expect(
      host.getWorkflowSchedule("node-a", first.selection_epoch, "schedule"),
    ).rejects.toThrow("Workspace changed");
    expect(fetch).toHaveBeenCalledTimes(2);
  });
  it("never retries mutations automatically and retains unknown-outcome evidence", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(response({ capabilities: caps }))
      .mockRejectedValueOnce(new TypeError("network lost"));
    vi.stubGlobal("fetch", fetch);
    const host = createWorkflowHost("project-a", ["read", "control"], () => []);
    const context = await host.workflowContext("node-a");
    const error = await host
      .setWorkflowScheduleEnabled(
        "node-a",
        context.selection_epoch,
        "schedule",
        false,
        "reviewed-etag",
      )
      .catch((error) => error);
    expect(error).toBeInstanceOf(RuntimeResourceFailure);
    expect(host.isOutcomeUnknown(error)).toBe(true);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(
      JSON.parse(fetch.mock.calls[1]![1].body).operation.request.etag,
    ).toBe("reviewed-etag");
  });
  it("preserves safe runtime validation detail and outcome certainty", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(response({ capabilities: caps }))
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            kind: "failed",
            connection_id: "connection-one",
            error: {
              message: "invalid request",
              outcome: "known",
              violations: [
                { description: "The first occurrence must be in the future." },
              ],
            },
          }),
        ),
      );
    vi.stubGlobal("fetch", fetch);
    const host = createWorkflowHost("project-a", ["read"], () => []);
    const context = await host.workflowContext("node-a");
    const error = await host
      .getWorkflowSchedule("node-a", context.selection_epoch, "schedule")
      .catch((error) => error);
    expect(error.message).toBe("The first occurrence must be in the future.");
    expect(host.isOutcomeUnknown(error)).toBe(false);
  });
  it("does not let a late context response replace a newer selection", async () => {
    let release: (response: Response) => void = () => {};
    const fetch = vi
      .fn()
      .mockImplementationOnce(
        () =>
          new Promise<Response>((resolve) => {
            release = resolve;
          }),
      )
      .mockResolvedValueOnce(response({ capabilities: caps }, "new-connection"))
      .mockResolvedValueOnce(
        response({ items: [], next_cursor: null }, "new-connection"),
      );
    vi.stubGlobal("fetch", fetch);
    const host = createWorkflowHost("project-a", ["read"], () => []);
    const old = host.workflowContext("node-a");
    const current = await host.workflowContext("node-a");
    release(response({ capabilities: caps }, "old-connection"));
    await expect(old).rejects.toThrow("workspace was refreshed");
    await host.listWorkflowSchedules("node-a", current.selection_epoch, null);
    expect(JSON.parse(fetch.mock.calls[2]![1].body).connection_id).toBe(
      "new-connection",
    );
  });
});
