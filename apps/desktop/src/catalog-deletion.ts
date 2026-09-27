import type { DeleteGlobalCatalogEntryRequest } from "./api";
import type { ManagedSettingsSnapshot } from "./types";

export type DeletableCatalogKind = "model" | "provider";

/** Display dependencies from the saved snapshot; native code rechecks on deletion. */
export function catalogDeletionBlockers(
  snapshot: ManagedSettingsSnapshot,
  kind: DeletableCatalogKind,
  resourceId: string,
): string[] {
  const global = snapshot.globalConfiguration;
  const profiles = new Set(
    kind === "provider"
      ? global.providers
          .find((entry) => entry.id === resourceId)
          ?.revisions.map(({ value }) => value.profile)
      : [],
  );
  const blockers = new Set<string>();
  for (const model of global.models.filter((entry) => !entry.archived)) {
    const current = model.revisions.find(
      ({ revision }) => revision === model.currentRevision,
    );
    if (current && profiles.has(current.value.providerProfile)) {
      blockers.add(`Model ${model.label}`);
    }
  }
  for (const space of snapshot.spaces) {
    const referenced = Object.entries(
      space.configuration.catalogRevisions,
    ).some(([key, reference]) => {
      if (key.startsWith(`${kind}:`) && reference.resourceId === resourceId)
        return true;
      const model = key.startsWith("model:")
        ? global.models
            .find((entry) => entry.id === reference.resourceId)
            ?.revisions.find(({ revision }) => revision === reference.revision)
        : undefined;
      return model !== undefined && profiles.has(model.value.providerProfile);
    });
    if (referenced)
      blockers.add(
        `Workspace ${space.name}${space.archived ? " (archived)" : ""}`,
      );
  }
  return [...blockers].sort();
}

/** Browser fixture counterpart to the native settings mutation. */
export function deleteCatalogEntryFixture(
  snapshot: ManagedSettingsSnapshot,
  kind: DeletableCatalogKind,
  request: DeleteGlobalCatalogEntryRequest,
): ManagedSettingsSnapshot {
  if (snapshot.globalConfiguration.revision !== request.expectedRevision) {
    throw new Error(
      "Global settings changed in another window. Reload and review the latest revision.",
    );
  }
  const key = kind === "model" ? "models" : "providers";
  if (
    !snapshot.globalConfiguration[key].some(
      (entry) => entry.id === request.resourceId,
    )
  ) {
    throw new Error(`The ${kind} is unknown.`);
  }
  if (catalogDeletionBlockers(snapshot, kind, request.resourceId).length) {
    throw new Error(
      `This ${kind} is still in use. Update its references first.`,
    );
  }
  const next = structuredClone(snapshot);
  const global = next.globalConfiguration;
  const defaults = global.defaults.revisions.find(
    ({ revision }) => revision === global.defaults.currentRevision,
  )!;
  global.revision += 1;
  global.defaults.currentRevision = global.revision;
  global.defaults.revisions.push({
    revision: global.revision,
    value: structuredClone(defaults.value),
  });
  if (kind === "model")
    global.models = global.models.filter(
      (entry) => entry.id !== request.resourceId,
    );
  else
    global.providers = global.providers.filter(
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
