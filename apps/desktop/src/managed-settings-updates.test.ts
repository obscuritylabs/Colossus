import { describe, expect, it, vi } from "vitest";
import type { ManagedSettingsSnapshot } from "./types";
const sync = vi.hoisted(() => vi.fn());
vi.mock("./api", () => ({ syncManagedConfiguration: sync }));
import {
  subscribeSettingsUpdates,
  syncSavedSettings,
} from "./managed-settings-updates";

describe("saved settings reconciliation", () => {
  it("shares in-flight polling with saves and publishes each result once", async () => {
    let finish!: (value: ManagedSettingsSnapshot) => void;
    sync.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const listener = vi.fn();
    const unsubscribe = subscribeSettingsUpdates(listener);
    const first = syncSavedSettings();
    expect(syncSavedSettings()).toBe(first);
    const snapshot = { spaces: [] } as unknown as ManagedSettingsSnapshot;
    finish(snapshot);
    await first;
    expect(listener).toHaveBeenCalledExactlyOnceWith(snapshot);
    unsubscribe();
    sync.mockResolvedValueOnce(snapshot);
    await syncSavedSettings();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("can reconcile again after a busy native operation without publishing a failed save", async () => {
    sync.mockRejectedValueOnce(new Error("busy"));
    await expect(syncSavedSettings()).rejects.toThrow("busy");
    sync.mockResolvedValueOnce(null);
    await expect(syncSavedSettings()).resolves.toBeNull();
  });
});
