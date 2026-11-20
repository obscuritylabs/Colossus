import { useCallback, useEffect, useRef, useState } from "react";
import { listDesktopAlwaysOn } from "./api";
import type { AlwaysOnWorkspace } from "./types";

export const ALWAYS_ON_REFRESH_MS = 5_000;

export function useAlwaysOnInventory(scope: string | null) {
  const [snapshot, setSnapshot] = useState<{
    scope: string;
    rows: AlwaysOnWorkspace[] | null;
  } | null>(null);
  const read = useRef<symbol | null>(null);
  const refresh = useCallback(
    async (replacePending = false) => {
      if (scope === null || (!replacePending && read.current !== null)) return;
      const request = Symbol();
      read.current = request;
      try {
        const next = await listDesktopAlwaysOn();
        if (!Array.isArray(next)) throw new Error("Invalid agent inventory.");
        if (read.current === request) {
          setSnapshot({ scope, rows: next });
        }
      } catch {
        if (read.current === request) {
          setSnapshot({ scope, rows: null });
        }
      } finally {
        if (read.current === request) read.current = null;
      }
    },
    [scope],
  );

  useEffect(() => {
    if (scope === null) return;
    void refresh();
    const timer = window.setInterval(
      () => void refresh(),
      ALWAYS_ON_REFRESH_MS,
    );
    return () => {
      window.clearInterval(timer);
      read.current = null;
    };
  }, [scope, refresh]);

  const current = snapshot?.scope === scope ? snapshot : null;
  return {
    rows: current?.rows ?? null,
    failed: current?.rows === null,
    refresh,
  };
}
