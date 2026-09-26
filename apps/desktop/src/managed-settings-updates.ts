import { syncManagedConfiguration } from "./api";
import type { ManagedSettingsSnapshot } from "./types";

const listeners = new Set<(snapshot: ManagedSettingsSnapshot) => void>();
let syncing: Promise<ManagedSettingsSnapshot | null> | null = null;

/** Share one native reconciliation between the app's poll and settings saves. */
export function syncSavedSettings() {
  syncing ??= syncManagedConfiguration()
    .then((snapshot) => {
      if (snapshot) for (const listener of listeners) listener(snapshot);
      return snapshot;
    })
    .finally(() => {
      syncing = null;
    });
  return syncing;
}

export function subscribeSettingsUpdates(
  listener: (snapshot: ManagedSettingsSnapshot) => void,
) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
