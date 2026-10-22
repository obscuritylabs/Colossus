import { useCallback, useMemo, useRef, useSyncExternalStore } from "react";

/** Personal navigation preferences, matching Desktop's bounded local pin list. */
export const MAX_PINNED_CHATS = 32;
const PREFIX = "colossus.web.chat-pins:v1:";
const MAX_STORED_LENGTH = 65_536;
const SESSION_NOTICE =
  "Browser storage is unavailable. Pins are kept for this session only.";
const LIMIT_NOTICE = `You can pin up to ${MAX_PINNED_CHATS} chats per project. Unpin a chat first.`;
const EMPTY_IDS: readonly string[] = Object.freeze([]);

interface Snapshot {
  ids: readonly string[];
  notice: string;
}
interface PinStore {
  getSnapshot: () => Snapshot;
  subscribe: (listener: () => void) => () => void;
  setPinned: (id: string, pinned: boolean) => void;
}
const EMPTY_SNAPSHOT: Snapshot = { ids: EMPTY_IDS, notice: "" };
const EMPTY_STORE: PinStore = {
  getSnapshot: () => EMPTY_SNAPSHOT,
  subscribe: () => () => {},
  setPinned: () => {},
};
const stores = new WeakMap<Window, Map<string, PinStore>>();

function identifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    value !== "." &&
    value !== ".." &&
    !/[\s/\\\u0000-\u001f\u007f]/u.test(value)
  );
}
function parse(serialized: string | null): readonly string[] {
  if (serialized === null || serialized.length > MAX_STORED_LENGTH)
    return EMPTY_IDS;
  try {
    const value: unknown = JSON.parse(serialized);
    if (
      typeof value !== "object" ||
      value === null ||
      !("version" in value) ||
      value.version !== 1 ||
      !("ids" in value) ||
      !Array.isArray(value.ids) ||
      Object.keys(value).some((key) => key !== "version" && key !== "ids")
    )
      return EMPTY_IDS;
    const ids: string[] = [];
    for (const id of value.ids) {
      if (identifier(id) && !ids.includes(id)) ids.push(id);
      if (ids.length === MAX_PINNED_CHATS) break;
    }
    return Object.freeze(ids);
  } catch {
    return EMPTY_IDS;
  }
}

function createStore(host: Window, key: string): PinStore {
  function read(): { ids: readonly string[]; failed: boolean } {
    try {
      return { ids: parse(host.localStorage.getItem(key)), failed: false };
    } catch {
      return { ids: EMPTY_IDS, failed: true };
    }
  }
  const initial = read();
  let sessionOnly = initial.failed;
  let snapshot: Snapshot = {
    ids: initial.ids,
    notice: sessionOnly ? SESSION_NOTICE : "",
  };
  const listeners = new Set<() => void>();
  function publish(ids: readonly string[], notice: string) {
    if (
      snapshot.notice === notice &&
      snapshot.ids.length === ids.length &&
      snapshot.ids.every((id, index) => id === ids[index])
    )
      return;
    snapshot = { ids: Object.freeze([...ids]), notice };
    for (const listener of listeners) listener();
  }
  function refresh() {
    // Once a write is unavailable, retain this session's choices without later
    // overwriting another tab with an incomplete durable merge. Reload to retry
    // browser persistence after its storage policy or quota has changed.
    if (sessionOnly) return;
    const latest = read();
    if (latest.failed) {
      sessionOnly = true;
      publish(snapshot.ids, SESSION_NOTICE);
    } else {
      publish(latest.ids, "");
    }
  }
  function storage(event: StorageEvent) {
    if (event.key !== key && event.key !== null) return;
    if (event.storageArea) {
      try {
        if (event.storageArea !== host.localStorage) return;
      } catch {
        refresh();
        return;
      }
    }
    // Read current storage instead of replaying possibly delayed event.newValue.
    refresh();
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      if (!listeners.size) host.addEventListener("storage", storage);
      listeners.add(listener);
      refresh();
      return () => {
        listeners.delete(listener);
        if (!listeners.size) host.removeEventListener("storage", storage);
      };
    },
    setPinned(id, pinned) {
      if (!identifier(id)) return;
      let ids = snapshot.ids;
      if (!sessionOnly) {
        // A tab may not have received another tab's storage event yet.
        const latest = read();
        if (latest.failed) sessionOnly = true;
        else ids = latest.ids;
      }
      const notice = sessionOnly ? SESSION_NOTICE : "";
      if (pinned && !ids.includes(id) && ids.length >= MAX_PINNED_CHATS) {
        publish(ids, [LIMIT_NOTICE, notice].filter(Boolean).join(" "));
        return;
      }
      if (pinned === ids.includes(id)) {
        publish(ids, notice);
        return;
      }
      const next = pinned ? [id, ...ids] : ids.filter((value) => value !== id);
      if (!sessionOnly) {
        try {
          host.localStorage.setItem(
            key,
            JSON.stringify({ version: 1, ids: next }),
          );
        } catch {
          sessionOnly = true;
        }
      }
      publish(next, sessionOnly ? SESSION_NOTICE : "");
    },
  };
}
function storeFor(identity: string, project: string): PinStore {
  if (
    typeof window === "undefined" ||
    !identifier(identity) ||
    !identifier(project)
  )
    return EMPTY_STORE;
  const key = PREFIX + JSON.stringify([identity, project]);
  let scopes = stores.get(window);
  if (!scopes) {
    scopes = new Map();
    stores.set(window, scopes);
  }
  let store = scopes.get(key);
  if (!store) {
    store = createStore(window, key);
    scopes.set(key, store);
  }
  return store;
}

/**
 * Browser-local pin IDs for exactly one signed-in account and project. Pins grant
 * no access; the host must resolve each ID through its existing authorized APIs.
 */
export function useChatPins(identity: string, project: string) {
  const store = useMemo(() => storeFor(identity, project), [identity, project]);
  const scope = useRef(store);
  // Fence old row/menu handlers during the first render of another account or
  // project, before effect cleanup or keyed consumers have had a chance to run.
  scope.current = store;
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    EMPTY_STORE.getSnapshot,
  );
  const setPinned = useCallback(
    (id: string, pinned: boolean) => {
      if (scope.current === store) store.setPinned(id, pinned);
    },
    [store],
  );
  return { ids: snapshot.ids, setPinned, notice: snapshot.notice };
}
