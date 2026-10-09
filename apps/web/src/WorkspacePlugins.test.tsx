// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { WorkspacePlugins } from "./WorkspaceResources";
import type { FleetNode } from "./api";

it("loads plugin discovery using the capability advertised by the public SDK", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          kind: "result",
          connection_id: "connection",
          value: { capabilities: ["plugins.discovery"] },
        }),
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          kind: "result",
          value: [
            {
              digest: "sha256:fixture",
              available: true,
              status: "available",
              manifest: {
                name: "authorized-plugin",
                version: "1.0.0",
                description: "Released discovery metadata",
              },
              skills: [],
            },
          ],
        }),
      ),
    );
  vi.stubGlobal("fetch", fetch);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const agent: FleetNode = {
    node: {
      node_id: "node",
      project_id: "project",
      instance_id: "instance",
      label: "Workspace",
      certificate_sha256: "fixture",
      roles: ["primary"],
      revoked: false,
      revision: 1,
    },
    presence: {
      ready: true,
      connection_id: "connection",
      capabilities: ["runtime.resources.v1"],
    },
  };
  try {
    await act(async () =>
      root.render(<WorkspacePlugins project="project" agent={agent} />),
    );
    expect(container.textContent).toContain("authorized-plugin");
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(JSON.parse(fetch.mock.calls[1]![1].body)).toEqual({
      connection_id: "connection",
      operation: { operation: "list_plugins" },
    });
  } finally {
    await act(() => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  }
});
