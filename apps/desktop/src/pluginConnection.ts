import type {
  ManagedSettingsSnapshot,
  SaveSpaceConfigurationRequest,
} from "./types";

function record(value: unknown): Record<string, unknown> {
  if (value && typeof value === "object" && !Array.isArray(value))
    return value as Record<string, unknown>;
  throw new Error("The effective plugin connection settings are invalid.");
}

export function pluginConnectionRequest(
  snapshot: ManagedSettingsSnapshot,
  spaceId: string,
  serverId: string,
  enabled: boolean,
  workspacePluginDigest?: string,
): SaveSpaceConfigurationRequest {
  const space = snapshot.spaces.find((candidate) => candidate.id === spaceId);
  if (!space || space.archived)
    throw new Error("The selected workspace is no longer available.");
  const current = space.configuration.fieldOverrides.find(
    (entry) => entry.fieldId === "plugins.mcpServers",
  );
  const servers = {
    ...record(current?.value ?? { $colossusPatchV1: true }),
  };
  const existing = record(servers[serverId] ?? {});
  // A connection may release credentials only to the explicitly selected source.
  // Changing between installed/local sources or local snapshots starts fresh.
  const sameSource =
    (existing.workspacePluginDigest ?? null) ===
    (workspacePluginDigest ?? null);
  servers[serverId] = {
    ...(sameSource ? existing : {}),
    enabled,
    workspacePluginDigest: workspacePluginDigest ?? null,
    allowedTools: enabled
      ? ["*"]
      : sameSource
        ? (existing.allowedTools ?? [])
        : [],
  };
  const fields = space.configuration.fieldOverrides.filter(
    (entry) => entry.fieldId !== "plugins.mcpServers",
  );
  fields.push({ fieldId: "plugins.mcpServers", value: servers });
  const selections = Object.entries(space.configuration.catalogRevisions);
  const selected = (kind: string) =>
    selections
      .filter(([key]) => key.startsWith(`${kind}:`))
      .map(([, entry]) => entry.resourceId);
  return {
    expectedGlobalRevision: snapshot.globalConfiguration.revision,
    spaceId,
    accessProfileOverride: space.configuration.accessProfileOverride,
    executionBoundaryOverride: space.configuration.executionBoundaryOverride,
    terminalEnabledOverride: space.configuration.terminalEnabledOverride,
    fieldOverrides: fields,
    selectedProviderResourceIds: selected("provider"),
    selectedModelResourceIds: selected("model"),
    selectedMcpResourceIds: selected("mcp"),
    selectedSearchResourceIds: selected("search"),
    selectedTelemetryResourceId: selected("telemetry")[0] ?? null,
    searchRoles: space.configuration.searchRoles,
    modelRoles: space.configuration.modelRoles,
    credentialOverrides: space.configuration.credentialOverrides,
  };
}
