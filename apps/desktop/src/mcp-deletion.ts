import type { DeleteGlobalMcpServerRequest } from "./api";
import type { ManagedSettingsSnapshot } from "./types";

export function managedMcpConsumers(
  snapshot: ManagedSettingsSnapshot,
  resourceId: string,
) {
  return snapshot.spaces.filter((space) =>
    Object.entries(space.configuration.catalogRevisions).some(
      ([key, reference]) =>
        key.startsWith("mcp:") && reference.resourceId === resourceId,
    ),
  );
}

/** Browser fixture counterpart to the native settings mutation. */
export function deleteMcpFixture(
  snapshot: ManagedSettingsSnapshot,
  request: DeleteGlobalMcpServerRequest,
): ManagedSettingsSnapshot {
  if (snapshot.globalConfiguration.revision !== request.expectedRevision) {
    throw new Error(
      "Global settings changed in another window. Reload and review the latest revision.",
    );
  }
  if (
    !snapshot.globalConfiguration.mcpServers.some(
      (entry) => entry.id === request.resourceId,
    )
  ) {
    throw new Error("The MCP server is unknown.");
  }
  if (managedMcpConsumers(snapshot, request.resourceId).length) {
    throw new Error(
      "Disable this MCP server and apply changes in every referencing workspace first.",
    );
  }
  const next = structuredClone(snapshot);
  const global = next.globalConfiguration;
  const defaults = global.defaults.revisions.find(
    (revision) => revision.revision === global.defaults.currentRevision,
  )!;
  global.revision += 1;
  global.defaults.currentRevision = global.revision;
  global.defaults.revisions.push({
    revision: global.revision,
    value: structuredClone(defaults.value),
  });
  global.mcpServers = global.mcpServers.filter(
    (entry) => entry.id !== request.resourceId,
  );
  for (const space of next.spaces) {
    if (
      space.configuration.acceptedGlobalRevision === request.expectedRevision
    ) {
      space.configuration.acceptedGlobalRevision = global.revision;
    } else {
      space.pendingGlobalRevision = global.revision;
      space.status = "update_available";
      space.statusMessage = `Global revision ${global.revision} is ready to review and apply.`;
    }
  }
  return next;
}
