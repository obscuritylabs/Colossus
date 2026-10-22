import { useCallback, useEffect, useState } from "react";
import { projectPath, request, type Thread } from "./api";

interface PinnedChatState {
  scope: string;
  threads: Thread[];
  unavailable: string[];
  loading: boolean;
}

/** Pins store IDs only; fresh authenticated metadata keeps older chats reachable. */
export function usePinnedChats(
  identity: string,
  project: string,
  ids: readonly string[],
  refreshKey: number,
) {
  const scope = JSON.stringify([identity, project]);
  const idsKey = JSON.stringify(ids);
  const [generation, setGeneration] = useState(0);
  const [state, setState] = useState<PinnedChatState>({
    scope,
    threads: [],
    unavailable: [],
    loading: ids.length > 0,
  });
  useEffect(() => {
    const targets: string[] = JSON.parse(idsKey);
    const alive = new AbortController();
    let pending = false;
    setState((current) => ({
      scope,
      threads:
        current.scope === scope
          ? current.threads.filter((thread) =>
              targets.includes(thread.thread_id),
            )
          : [],
      unavailable: [],
      loading: targets.length > 0,
    }));
    if (!targets.length) return () => alive.abort();
    let active: AbortController | null = null;
    const load = async () => {
      if (pending) return;
      pending = true;
      const abort = new AbortController();
      active = abort;
      const deadline = setTimeout(() => abort.abort(), 10_000);
      const threads = new Map<string, Thread>();
      let next = 0;
      const worker = async () => {
        while (!abort.signal.aborted && !alive.signal.aborted) {
          const id = targets[next++];
          if (!id) return;
          try {
            const { thread } = await request<{ thread: Thread }>(
              `${projectPath(project)}/threads/${encodeURIComponent(id)}/metadata`,
              undefined,
              abort.signal,
            );
            if (
              !abort.signal.aborted &&
              !alive.signal.aborted &&
              thread.thread_id === id &&
              thread.project_id === project &&
              thread.owner_user_id === identity &&
              thread.source === "cloud"
            )
              threads.set(id, thread);
          } catch {
            // Keep unavailable IDs removable without retaining private chat content.
          }
        }
      };
      try {
        await Promise.all(
          Array.from({ length: Math.min(4, targets.length) }, worker),
        );
        if (!alive.signal.aborted)
          setState({
            scope,
            threads: targets.flatMap((id) => {
              const thread = threads.get(id);
              return thread ? [thread] : [];
            }),
            unavailable: targets.filter((id) => !threads.has(id)),
            loading: false,
          });
      } finally {
        clearTimeout(deadline);
        pending = false;
      }
    };
    void load();
    const timer = setInterval(() => {
      if (!document.hidden) void load();
    }, 30_000);
    const focus = () => void load();
    window.addEventListener("focus", focus);
    return () => {
      alive.abort();
      active?.abort();
      clearInterval(timer);
      window.removeEventListener("focus", focus);
    };
  }, [scope, identity, project, idsKey, generation, refreshKey]);
  const refresh = useCallback(() => setGeneration((value) => value + 1), []);
  return {
    threads:
      state.scope === scope
        ? state.threads.filter((thread) => ids.includes(thread.thread_id))
        : [],
    unavailable:
      state.scope === scope
        ? state.unavailable.filter((id) => ids.includes(id))
        : [],
    loading: state.scope === scope ? state.loading : ids.length > 0,
    refresh,
  };
}
