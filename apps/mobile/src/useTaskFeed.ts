import { useEffect, useRef, useState } from "react";
import type { Update } from "@colossus/control-plane-client";
import { messageOf, type MobileClient, type MobileStreamEvent } from "./client";
import { FeedWindow, streamCursor } from "./feed-window";

type FeedState = {
  scope: string;
  updates: Update[];
  error: string;
  limited: boolean;
  connected: boolean;
  loading: boolean;
};

/** Native custody owns the stream. Visibility releases it and resumes its cursor. */
export function useTaskFeed(
  client: MobileClient,
  project: string,
  task: string,
  options: { thread?: string; onThreadChanged?: () => void } = {},
) {
  const thread = options.thread;
  const scope = JSON.stringify([project, task, thread]);
  const [state, setState] = useState<FeedState>({
    scope,
    updates: [],
    error: "",
    limited: false,
    connected: false,
    loading: Boolean(task || thread),
  });
  const wake = useRef<() => void>(() => {});
  const changed = useRef(options.onThreadChanged);
  changed.current = options.onThreadChanged;
  useEffect(() => {
    let alive = true,
      generation = 0,
      reading = false,
      subscribing = false,
      connected = false,
      retainedCursor = 0,
      cursor = "0";
    let stop: (() => Promise<void>) | undefined;
    let retryBlocked = false;
    const window = new FeedWindow();
    const publish = (patch: Partial<FeedState> = {}) => {
      if (alive)
        setState((previous) => ({
          ...previous,
          scope,
          updates: window.values(),
          limited: window.limited,
          ...patch,
        }));
    };
    publish({ error: "", connected: false, loading: Boolean(task || thread) });
    async function retained() {
      if (!alive || !task || reading || document.hidden) return;
      reading = true;
      try {
        for (let batch = 0; batch < 4 && alive && !document.hidden; batch++) {
          const after = retainedCursor;
          const page = await client.request<{
            updates: Update[];
            next_after: number | null;
          }>({
            kind: "task_updates",
            project,
            task,
            after,
          });
          if (!alive) return;
          // A live update can overtake this read. Its cursor already covers the
          // old page, so never put evicted replay back into the recent window.
          if (retainedCursor !== after) continue;
          for (const update of page.updates) {
            if (update.task_id && update.task_id !== task)
              throw new Error(
                "The runtime returned activity for another task.",
              );
            if (update.sequence <= retainedCursor)
              throw new Error("Task activity returned an invalid sequence.");
            window.append({ ...update, task_id: update.task_id ?? task });
            retainedCursor = update.sequence;
          }
          publish({ error: "", loading: false });
          if (page.next_after == null) break;
          if (!page.updates.length || page.next_after !== retainedCursor)
            throw new Error("Task activity returned an invalid cursor.");
        }
      } catch (error) {
        publish({ error: messageOf(error), loading: false });
      } finally {
        reading = false;
      }
    }
    async function subscribe() {
      if (
        !alive ||
        !(thread || task) ||
        document.hidden ||
        subscribing ||
        stop ||
        retryBlocked
      )
        return;
      subscribing = true;
      const epoch = generation;
      let ended = false;
      try {
        const unsubscribe = await client.watch(
          { project, ...(thread ? { thread } : { task }), after: cursor },
          (event: MobileStreamEvent) => {
            if (!alive || epoch !== generation) return;
            if (event.kind === "connection") {
              connected = event.connected;
              publish({ connected, loading: false });
            } else if (event.kind === "run_update") {
              try {
                const nextCursor = streamCursor(event.cursor);
                if (BigInt(nextCursor) < BigInt(cursor)) return;
                if (
                  task &&
                  !thread &&
                  event.update.task_id &&
                  event.update.task_id !== task
                )
                  throw new Error(
                    "The runtime returned activity for another task.",
                  );
                const update =
                  task && !thread
                    ? { ...event.update, task_id: task }
                    : event.update;
                window.append(update);
                cursor = nextCursor;
                if (task && !thread)
                  retainedCursor = Math.max(retainedCursor, update.sequence);
                publish({ error: "", loading: false });
                if (
                  event.update.update.result ||
                  event.update.update.state ||
                  event.update.update.interaction ||
                  event.update.update.message ||
                  event.update.update.created ||
                  event.update.update.started ||
                  event.update.update.failure ||
                  event.update.update.cancellation
                )
                  changed.current?.();
              } catch (error) {
                publish({ error: messageOf(error), loading: false });
              }
            } else if (event.kind === "thread_changed") {
              try {
                const nextCursor = streamCursor(event.cursor);
                if (BigInt(nextCursor) < BigInt(cursor)) return;
                cursor = nextCursor;
                changed.current?.();
              } catch (error) {
                publish({ error: messageOf(error), loading: false });
              }
            } else if (event.kind === "error") {
              ended = true;
              retryBlocked = [
                "session_expired",
                "permission_denied",
                "forbidden",
                "disconnected",
              ].includes(event.code);
              const unsubscribe = stop;
              stop = undefined;
              if (unsubscribe) void unsubscribe().catch(() => {});
              connected = false;
              publish({
                error: event.message,
                connected: false,
                loading: false,
              });
            }
          },
        );
        if (!alive || epoch !== generation || ended) await unsubscribe();
        else stop = unsubscribe;
      } catch (error) {
        publish({ error: messageOf(error), connected: false, loading: false });
      } finally {
        subscribing = false;
      }
    }
    function resume() {
      if (document.hidden) {
        generation++;
        connected = false;
        const unsubscribe = stop;
        stop = undefined;
        if (unsubscribe) void unsubscribe().catch(() => {});
        publish({ connected: false });
      } else {
        void subscribe();
        if (!connected) void retained();
        changed.current?.();
      }
    }
    wake.current = () => {
      void retained();
      void subscribe();
    };
    void retained();
    void subscribe();
    const timer = globalThis.setInterval(() => {
      if (!document.hidden && !connected) {
        void retained();
        void subscribe();
      }
    }, 5000);
    document.addEventListener("visibilitychange", resume);
    globalThis.addEventListener("focus", resume);
    return () => {
      alive = false;
      generation++;
      wake.current = () => {};
      globalThis.clearInterval(timer);
      document.removeEventListener("visibilitychange", resume);
      globalThis.removeEventListener("focus", resume);
      if (stop) void stop().catch(() => {});
    };
  }, [client, project, task, thread, scope]);
  const current = state.scope === scope;
  return {
    updates: current ? state.updates : [],
    error: current ? state.error : "",
    limited: current && state.limited,
    connected: current && state.connected,
    loading: current ? state.loading : Boolean(task || thread),
    refresh: () => wake.current(),
  };
}
