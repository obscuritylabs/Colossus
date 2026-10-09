// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Fleet } from "./Fleet";
import type { Host, FleetNode } from "./api";
it("renders one compact host table row with workspace names, OS, and accurate connected counts", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host: Host = {
    host_id: "host",
    project_id: "project",
    label: "Workstation",
    platform: "macos",
    deployment_kind: "desktop",
    revision: 1,
    last_seen_at: 1,
  };
  const node = (id: string, ready: boolean, revoked = false): FleetNode => ({
    node: {
      node_id: id,
      project_id: "project",
      host_id: "host",
      instance_id: id,
      label: id,
      workspace_label: id,
      certificate_sha256: "test",
      roles: ["primary"],
      revoked,
      revision: 1,
    },
    presence: { ready, connection_id: id, capabilities: [] },
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container),
    open = vi.fn();
  try {
    await act(() =>
      root.render(
        <Fleet
          nodes={[
            node("Colossus", true),
            node("pointbreak", false),
            node("revoked", true, true),
          ]}
          hosts={[host, { ...host, host_id: "foreign", project_id: "other" }]}
          project="project"
          permissions={[]}
          onRefresh={vi.fn()}
          onError={vi.fn()}
          hasMore={false}
          hasMoreHosts={false}
          onMore={vi.fn()}
          onMoreHosts={vi.fn()}
          onOpenAgent={vi.fn()}
          onOpenHost={open}
        />,
      ),
    );
    const table = container.querySelector("table")!;
    expect(table.getAttribute("aria-label")).toBe("Fleet hosts");
    expect(table.textContent).toContain("macOS");
    expect(table.textContent).toContain("Colossus · pointbreak");
    expect(table.textContent).toContain("1 of 2");
    expect(table.querySelectorAll("tbody tr")).toHaveLength(1);
    expect(container.querySelector(".fleet-host-list")).toBeNull();
    const link = container.querySelector<HTMLAnchorElement>(
      'a[aria-label="Open host Workstation"]',
    )!;
    expect(link.getAttribute("href")).toBe("/projects/project/hosts/host");
    await act(() => link.click());
    expect(open).toHaveBeenCalledExactlyOnceWith("host");
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});
