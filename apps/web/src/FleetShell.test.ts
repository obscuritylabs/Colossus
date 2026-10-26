import { expect, it } from "vitest";
import { fleetShellAvailable } from "./FleetShell";
import type { FleetNode } from "./api";
const node: FleetNode = {
  node: {
    node_id: "node",
    project_id: "project",
    instance_id: "instance",
    label: "Workspace",
    certificate_sha256: "test",
    roles: ["primary"],
    revoked: false,
    revision: 1,
  },
  presence: {
    ready: true,
    capabilities: ["human_terminal.v1"],
    connection_id: "connection",
  },
};
it("requires explicit terminal permission and the selected connected workspace advertisement", () => {
  expect(
    fleetShellAvailable("project", node, [
      "read",
      "execute",
      "control",
      "administer",
    ]),
  ).toBe(false);
  expect(fleetShellAvailable("project", node, ["terminal"])).toBe(true);
  expect(fleetShellAvailable("other", node, ["terminal"])).toBe(false);
  expect(
    fleetShellAvailable(
      "project",
      { ...node, node: { ...node.node, revoked: true } },
      ["terminal"],
    ),
  ).toBe(false);
  expect(
    fleetShellAvailable(
      "project",
      { ...node, node: { ...node.node, execution_kind: "hosted" } },
      ["terminal"],
    ),
  ).toBe(false);
  expect(
    fleetShellAvailable("project", { ...node, presence: null }, ["terminal"]),
  ).toBe(false);
  expect(
    fleetShellAvailable(
      "project",
      { ...node, presence: { ...node.presence!, ready: false } },
      ["terminal"],
    ),
  ).toBe(false);
  expect(
    fleetShellAvailable(
      "project",
      { ...node, presence: { ...node.presence!, capabilities: [] } },
      ["terminal"],
    ),
  ).toBe(false);
});
