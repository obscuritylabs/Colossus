import { expect, it } from "vitest";
import {
  hostConnection,
  hostWorkspaces,
  workspaceName,
} from "./workspace-navigation";
import type { FleetNode, Host } from "./api";
it("groups by stable project/host IDs and uses workspace names independently of aliases", () => {
  const host: Host = {
    host_id: "h",
    project_id: "p",
    label: "Same display name",
    platform: "macos",
    deployment_kind: "desktop",
    last_seen_at: null,
    revision: 1,
  };
  const node = (
    id: string,
    project: string,
    hostId: string,
    ready: boolean,
  ): FleetNode => ({
    node: {
      node_id: id,
      project_id: project,
      host_id: hostId,
      workspace_label: `Workspace ${id}`,
      label: "Alias",
      instance_id: id,
      certificate_sha256: "test",
      roles: [],
      revoked: false,
      revision: 1,
    },
    presence: { ready, connection_id: id, capabilities: [] },
  });
  const nodes = [
    node("a", "p", "h", true),
    node("b", "p", "h", false),
    node("c", "q", "h", true),
    node("d", "p", "other", true),
  ];
  expect(
    hostWorkspaces(nodes, "p", "h").map((item) => item.node.node_id),
  ).toEqual(["a", "b"]);
  expect(hostConnection(host, nodes)).toEqual({
    ready: 1,
    total: 2,
    label: "Connected",
  });
  expect(workspaceName(nodes[0]!)).toBe("Workspace a");
});
