import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Task,
  ThreadDetailResponse,
  ThreadMessage,
} from "@colossus/control-plane-client";
import { messageOf, type MobileClient } from "./client";

function merge<T>(current: T[], incoming: T[], id: (item: T) => string) {
  const items = new Map(current.map((item) => [id(item), item]));
  for (const item of incoming) items.set(id(item), item);
  return [...items.values()];
}
/** Keep loaded history while the live stream refreshes the latest page. */
export function useThreadHistory(
  client: MobileClient,
  project: string,
  thread?: string,
) {
  const [value, setValue] = useState<ThreadDetailResponse | null>(null);
  const [error, setError] = useState("");
  const [loadingMore, setLoadingMore] = useState(false);
  const scope = JSON.stringify([project, thread ?? ""]);
  const active = useRef({ scope, client, generation: 0 });
  const state = useRef<ThreadDetailResponse | null>(null);
  const loaded = useRef(false);
  const pending = useRef(false);
  const read = useRef<() => void>(() => {});
  useEffect(() => {
    const generation = active.current.generation + 1;
    active.current = { scope, client, generation };
    state.current = null;
    loaded.current = false;
    pending.current = false;
    setValue(null);
    setError("");
    setLoadingMore(false);
    let alive = true,
      refreshing = false;
    async function refresh() {
      if (!thread || !alive || refreshing || document.hidden) return;
      refreshing = true;
      try {
        const page = await client.request<ThreadDetailResponse>({
          kind: "thread",
          project,
          thread,
        });
        if (!alive) return;
        if (
          page.thread.active_task_id &&
          !page.tasks.some(
            (task) => task.task_id === page.thread.active_task_id,
          )
        ) {
          const { task } = await client.request<{ task: Task }>({
            kind: "task",
            project,
            task: page.thread.active_task_id,
          });
          if (!alive) return;
          page.tasks.push(task);
        }
        const prior = state.current;
        const next = {
          ...page,
          messages: merge(
            prior?.messages ?? [],
            page.messages,
            (message: ThreadMessage) => message.message_id,
          ),
          tasks: merge(
            prior?.tasks ?? [],
            page.tasks,
            (task: Task) => task.task_id,
          ),
          message_authors: {
            ...prior?.message_authors,
            ...page.message_authors,
          },
          next_message_cursor: loaded.current
            ? (prior?.next_message_cursor ?? null)
            : (page.next_message_cursor ?? null),
          next_task_cursor: loaded.current
            ? (prior?.next_task_cursor ?? null)
            : (page.next_task_cursor ?? null),
        };
        state.current = next;
        setValue(next);
        setError("");
      } catch (failure) {
        if (alive) setError(messageOf(failure));
      } finally {
        refreshing = false;
      }
    }
    read.current = () => void refresh();
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    const wake = () => void refresh();
    window.addEventListener("focus", wake);
    document.addEventListener("visibilitychange", wake);
    return () => {
      alive = false;
      clearInterval(timer);
      window.removeEventListener("focus", wake);
      document.removeEventListener("visibilitychange", wake);
    };
  }, [client, project, thread, scope]);
  const refresh = useCallback(() => read.current(), []);
  async function loadMore() {
    const prior = state.current;
    if (
      !thread ||
      pending.current ||
      !prior ||
      !(prior.next_message_cursor || prior.next_task_cursor)
    )
      return;
    const generation = active.current.generation;
    pending.current = true;
    setLoadingMore(true);
    try {
      const page = await client.request<ThreadDetailResponse>({
        kind: "thread",
        project,
        thread,
        ...(prior.next_message_cursor
          ? { message_after: prior.next_message_cursor }
          : {}),
        ...(prior.next_task_cursor
          ? { task_after: prior.next_task_cursor }
          : {}),
      });
      if (generation !== active.current.generation) return;
      const latest = state.current ?? prior;
      const next = {
        ...latest,
        messages: merge(
          page.messages,
          latest.messages,
          (message) => message.message_id,
        ),
        tasks: merge(page.tasks, latest.tasks, (task) => task.task_id),
        message_authors: { ...page.message_authors, ...latest.message_authors },
        next_message_cursor: prior.next_message_cursor
          ? (page.next_message_cursor ?? null)
          : null,
        next_task_cursor: prior.next_task_cursor
          ? (page.next_task_cursor ?? null)
          : null,
      };
      loaded.current = true;
      state.current = next;
      setValue(next);
      setError("");
    } catch (failure) {
      if (generation === active.current.generation)
        setError(messageOf(failure));
    } finally {
      if (generation === active.current.generation) {
        pending.current = false;
        setLoadingMore(false);
      }
    }
  }
  return {
    value:
      active.current.scope === scope && active.current.client === client
        ? value
        : null,
    error,
    refresh,
    loadMore,
    loadingMore,
  };
}
