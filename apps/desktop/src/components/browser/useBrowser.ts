import { useCallback, useEffect, useRef, useState } from "react";
import { useAppearance } from "../../theme/AppearanceProvider";
import { browserErrorMessage, nativeBrowserApi } from "../../browser-api";
import type {
  BrowserAction,
  BrowserCertificateAction,
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
  conversationId?: string,
) {
  const appearance = useAppearance();
  const appearanceRef = useRef(appearance);
  appearanceRef.current = appearance;
  const [snapshot, setSnapshot] = useState<BrowserSnapshot>({
    ...empty,
    available: fixture,
  });
  const [error, setError] = useState("");
  const [contextError, setContextError] = useState("");
  const [loading, setLoading] = useState(!fixture);
  const [busy, setBusy] = useState(false);
  const api = useRef<BrowserApi>(nativeBrowserApi);
  const conversation = useRef(conversationId);
  conversation.current = conversationId;
  const sequence = useRef(0);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;

  useEffect(() => {
    setSnapshot({ ...empty, available: fixture });
    setLoading(!fixture);
    setError("");
    setContextError("");
    setBusy(false);
  }, [scope, fixture]);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const capturedScope = scope;
    async function load() {
      if (fixture && import.meta.env.DEV) {
        const { browserFixture } = await import("../../dev/browser-fixture");
        api.current = browserFixture(capturedScope ?? "fixture");
      } else api.current = nativeBrowserApi;
      async function refresh() {
        const started = sequence.current;
        const current = () =>
          !cancelled &&
          currentScope.current === capturedScope &&
          sequence.current === started;
        try {
          const value = await api.current.context();
          if (current()) {
            setSnapshot(value);
            setLoading(false);
            setContextError("");
          }
        } catch (cause) {
          if (current()) {
            setLoading(false);
            setContextError(browserErrorMessage(cause));
          }
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
    const current = () =>
      currentScope.current === capturedScope && operation === sequence.current;
    setBusy(true);
    setError("");
    try {
      const result = await api.current.command(
        snapshotRef.current.generation,
        action.type === "new" && !action.conversationId && conversation.current
          ? { ...action, conversationId: conversation.current }
          : action,
      );
      if (current()) setSnapshot(result);
    } catch (cause) {
      if (current()) setError(browserErrorMessage(cause));
    } finally {
      if (current()) setBusy(false);
    }
  }, []);

  const viewport = useCallback(
    (request: BrowserViewport) => api.current.viewport(request).catch(() => {}),
    [],
  );
  const certificates = useCallback(async (action: BrowserCertificateAction) => {
    const generation = snapshotRef.current.generation;
    const capturedScope = currentScope.current;
    const handler = api.current.certificates;
    if (!handler) throw new Error("Certificate setup is unavailable.");
    const status = await handler(
      generation,
      action,
      {
        colorScheme: appearanceRef.current.resolvedColorTheme,
        textSize: appearanceRef.current.textSize,
      },
      action === "review_client_identity"
        ? (snapshotRef.current.selectedTabId ?? undefined)
        : undefined,
    );
    if (capturedScope !== currentScope.current)
      throw new Error("The selected workspace changed.");
    const value = await api.current.context();
    if (capturedScope === currentScope.current) setSnapshot(value);
    return status;
  }, []);
  return {
    snapshot,
    error: error || contextError,
    loading,
    busy,
    command,
    viewport,
    certificates,
    fixture,
  };
}

export type BrowserController = ReturnType<typeof useBrowser>;
