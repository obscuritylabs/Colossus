import { useCallback, useEffect, useRef, useState } from "react";
import { browserErrorMessage, nativeBrowserApi } from "../../browser-api";
import type {
  BrowserAction,
  BrowserApi,
  BrowserSnapshot,
  BrowserViewport,
} from "../../browser-api";

const empty: BrowserSnapshot = {
  available: false,
  generation: 0,
  tabs: [],
  selectedTabId: null,
};

export function useBrowser(
  scope: string | null,
  visible: boolean,
  fixture = false,
) {
  const [snapshot, setSnapshot] = useState<BrowserSnapshot>({
    ...empty,
    available: fixture,
  });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const api = useRef<BrowserApi>(nativeBrowserApi);
  const sequence = useRef(0);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    setSnapshot({ ...empty, available: fixture });
    setError("");
    setBusy(false);
    const capturedScope = scope;
    async function load() {
      if (fixture && import.meta.env.DEV) {
        const { browserFixture } = await import("../../dev/browser-fixture");
        api.current = browserFixture(capturedScope ?? "fixture");
      } else api.current = nativeBrowserApi;
      async function refresh() {
        const started = sequence.current;
        try {
          const value = await api.current.context();
          if (
            !cancelled &&
            currentScope.current === capturedScope &&
            sequence.current === started
          )
            setSnapshot(value);
        } catch {
          /* Browsing is optional in builds without the native preview. */
        }
        if (!cancelled && visible)
          timer = window.setTimeout(() => void refresh(), 750);
      }
      if (!cancelled) await refresh();
    }
    void load();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      const current = snapshotRef.current;
      if (current.available)
        void api.current
          .viewport({ generation: current.generation, tabId: null, rect: null })
          .catch(() => {});
    };
  }, [scope, visible, fixture]);

  const command = useCallback(async (action: BrowserAction) => {
    const operation = ++sequence.current;
    const capturedScope = currentScope.current;
    setBusy(true);
    setError("");
    try {
      const result = await api.current.command(
        snapshotRef.current.generation,
        action,
      );
      if (
        currentScope.current === capturedScope &&
        operation === sequence.current
      )
        setSnapshot(result);
    } catch (cause) {
      if (
        currentScope.current === capturedScope &&
        operation === sequence.current
      )
        setError(browserErrorMessage(cause));
    } finally {
      if (
        currentScope.current === capturedScope &&
        operation === sequence.current
      )
        setBusy(false);
    }
  }, []);

  const viewport = useCallback(
    (request: BrowserViewport) => api.current.viewport(request).catch(() => {}),
    [],
  );
  return { snapshot, error, busy, command, viewport, fixture };
}

export type BrowserController = ReturnType<typeof useBrowser>;
