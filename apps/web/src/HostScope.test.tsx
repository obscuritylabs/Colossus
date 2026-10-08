// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HostSidebar } from "./HostScope";
import type { FleetNode, Host } from "./api";

const host: Host = {
  host_id: "host-a",
  project_id: "p",
  label: "Workstation",
  platform: "macos",
  deployment_kind: "desktop",
  revision: 1,
  last_seen_at: 1,
};
const node = (id: string, hostId: string, label: string): FleetNode => ({
  node: {
    node_id: id,
    project_id: "p",
    instance_id: id,
    label: "Legacy alias",
    workspace_label: label,
    host_id: hostId,
    certificate_sha256: "test",
    roles: ["primary"],
    revoked: false,
    revision: 1,
  },
  presence: { ready: true, connection_id: id, capabilities: [] },
});
afterEach(() => vi.unstubAllGlobals());
it("shows simultaneous host workspaces, scopes each thread query, and rejects foreign rows", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const paths: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string) => {
      paths.push(path);
      const id = new URL(path, "http://test").searchParams.get("node_id");
      const thread = (nodeId: string) => ({
        thread_id: `thread-${nodeId}`,
        project_id: "p",
        node_id: nodeId,
        title: `Thread ${nodeId}`,
        created_at: "2026-10-08T12:00:00Z",
        updated_at: "2026-10-08T12:00:00Z",
        archived: false,
        source: "cloud",
        can_continue: true,
      });
      return new Response(
        JSON.stringify({
          threads: [thread(id!), thread("foreign")],
          next_cursor: null,
        }),
      );
    }),
  );
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const select = vi.fn(),
    create = vi.fn();
  try {
    await act(() =>
      root.render(
        <HostSidebar
          project="p"
          host={host}
          nodes={[
            node("a", "host-a", "Colossus"),
            node("b", "host-a", "pointbreak"),
            node("c", "host-b", "Other host workspace"),
          ]}
          agentId="a"
          selected=""
          identity="user"
          canExecute={false}
          onAgent={select}
          onOpen={() => {}}
          onNew={create}
          onBack={() => {}}
          hasMore={false}
          onMore={() => {}}
          moreBusy={false}
          error=""
        />,
      ),
    );
    expect(element.textContent).toContain("Colossus");
    expect(element.textContent).toContain("pointbreak");
    expect(element.textContent).not.toContain("Other host workspace");
    expect(element.textContent).not.toContain("Legacy alias");
    expect(
      paths
        .map((path) => new URL(path, "http://test").searchParams.get("node_id"))
        .sort(),
    ).toEqual(["a", "b"]);
    expect(element.textContent).toContain("Thread a");
    expect(element.textContent).toContain("Thread b");
    expect(element.textContent).not.toContain("Thread foreign");
    expect(
      [
        ...element.querySelectorAll<HTMLButtonElement>(".scope-new-thread"),
      ].every((button) => button.disabled),
    ).toBe(true);
    await act(() =>
      element
        .querySelector<HTMLButtonElement>(
          'button[aria-label="Collapse pointbreak threads"]',
        )!
        .click(),
    );
    expect(element.textContent).not.toContain("Thread b");
    expect(select).not.toHaveBeenCalled();
    expect(create).not.toHaveBeenCalled();
  } finally {
    await act(() => root.unmount());
    element.remove();
  }
});
