import { useCallback, useEffect, useState } from "react";
import { request } from "./api";
interface ResourceState<T> {
  scope: string;
  data: T | null;
  error: string;
  loading: boolean;
}
/** The host supplies identity to fence caches across account changes at the same URL. */
export function useResource<T>(
  path: string | null,
  interval = 0,
  identity = "",
) {
  const scope = JSON.stringify([identity, path]);
  const [state, setState] = useState<ResourceState<T>>({
      scope,
      data: null,
      error: "",
      loading: Boolean(path),
    }),
    [generation, setGeneration] = useState(0);
  useEffect(() => {
    if (!path) {
      setState({ scope, data: null, error: "", loading: false });
      return;
    }
    const abort = new AbortController();
    let pending = false;
    setState((current) =>
      current.scope === scope
        ? { ...current, loading: current.data === null, error: "" }
        : { scope, data: null, error: "", loading: true },
    );
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const value = await request<T>(path, undefined, abort.signal);
        if (!abort.signal.aborted)
          setState({ scope, data: value, error: "", loading: false });
      } catch (e) {
        if (!abort.signal.aborted)
          setState((current) => ({
            scope,
            data: current.scope === scope ? current.data : null,
            error:
              e instanceof Error ? e.message : "Could not load this resource.",
            loading: false,
          }));
      } finally {
        pending = false;
      }
    };
    void load();
    const timer = interval
      ? setInterval(() => {
          if (!document.hidden) void load();
        }, interval)
      : null;
    return () => {
      abort.abort();
      if (timer) clearInterval(timer);
    };
  }, [path, scope, interval, generation]);
  const refresh = useCallback(() => setGeneration((value) => value + 1), []);
  const current =
    state.scope === scope
      ? state
      : { scope, data: null, error: "", loading: Boolean(path) };
  return {
    data: current.data,
    error: current.error,
    loading: current.loading,
    refresh,
  };
}
export function errorMessage(error: unknown) {
  return error instanceof Error
    ? error.message
    : "The change could not be saved.";
}
