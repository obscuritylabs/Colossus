import { describe, expect, it } from "vitest";
import type { ManagedSettingsSnapshot } from "./types";
import { pluginConnectionRequest } from "./pluginConnection";

describe("plugin connection save", () => {
  it("preserves unrelated workspace settings while explicitly allowing all plugin tools", () => {
    const snapshot = {
      globalConfiguration: { revision: 17 },
      spaces: [
        {
          id: "workspace",
          archived: false,
          effectiveValues: [
            {
              fieldId: "plugins.mcpServers",
              value: {
                "example/mail": { enabled: false, allowedTools: ["list_mail"] },
                "other/docs": { enabled: true, allowedTools: ["search"] },
              },
            },
          ],
          configuration: {
            accessProfileOverride: "pinned",
            executionBoundaryOverride: null,
            terminalEnabledOverride: false,
            fieldOverrides: [{ fieldId: "sandbox.profile", value: "strict" }],
            catalogRevisions: {
              "provider:p1": { resourceId: "p1", revision: 4 },
              "mcp:m1": { resourceId: "m1", revision: 2 },
            },
            searchRoles: { agent: "search-1" },
            modelRoles: { chat: "model-1" },
            credentialOverrides: { token: "cred-1" },
          },
        },
      ],
    } as unknown as ManagedSettingsSnapshot;

    const request = pluginConnectionRequest(
      snapshot,
      "workspace",
      "example/mail",
      true,
    );
    expect(request.expectedGlobalRevision).toBe(17);
    expect(request.selectedProviderResourceIds).toEqual(["p1"]);
    expect(request.selectedMcpResourceIds).toEqual(["m1"]);
    expect(request.credentialOverrides).toEqual({ token: "cred-1" });
    expect(request.fieldOverrides).toEqual([
      { fieldId: "sandbox.profile", value: "strict" },
      {
        fieldId: "plugins.mcpServers",
        value: {
          $colossusPatchV1: true,
          "example/mail": { enabled: true, allowedTools: ["*"] },
        },
      },
    ]);
  });

  it("keeps an existing whole-map workspace override as a whole-map override", () => {
    const snapshot = {
      globalConfiguration: { revision: 17 },
      spaces: [
        {
          id: "workspace",
          archived: false,
          configuration: {
            fieldOverrides: [
              {
                fieldId: "plugins.mcpServers",
                value: { "local/docs": { enabled: true } },
              },
            ],
            catalogRevisions: {},
            searchRoles: {},
            modelRoles: {},
            credentialOverrides: {},
          },
        },
      ],
    } as unknown as ManagedSettingsSnapshot;
    const request = pluginConnectionRequest(
      snapshot,
      "workspace",
      "local/docs",
      false,
    );
    expect(request.fieldOverrides).toEqual([
      {
        fieldId: "plugins.mcpServers",
        value: { "local/docs": { enabled: false, allowedTools: [] } },
      },
    ]);
  });
});
