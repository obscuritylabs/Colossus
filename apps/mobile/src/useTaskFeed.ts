import { useEffect, useRef, useState } from "react";
import type { Update } from "@colossus/control-plane-client";
import { messageOf, type MobileClient } from "./client";

const MAX_EVENTS = 1000;
const MAX_BYTES = 1024 * 1024;
/** Bounded retained-event catch-up. Mutations and live runtime reads stay outside this hook. */
export function useTaskFeed(
  client: MobileClient,
  project: string,
  task: string,
) {
  const [updates, setUpdates] = useState<Update[]>([]),
    [error, setError] = useState("");
  const [limited, setLimited] = useState(false);
  const wake = useRef<() => void>(() => {});
  useEffect(() => {
    let alive = true,
      pending = false,
      stopped = false,
      cursor = 0,
      bytes = 0;
    const events: Update[] = [];
    setUpdates([]);
    setError("");
    setLimited(false);
    async function read() {
      if (pending || document.hidden || stopped || !alive) return;
      pending = true;
      try {
        for (let batch = 0; batch < 4 && alive && !document.hidden; batch++) {
          const page = await client.request<{
            updates: Update[];
            next_after: number | null;
          }>({ kind: "task_updates", project, task, after: cursor });
          if (!alive) return;
          for (const event of page.updates) {
            if (
              !Number.isSafeInteger(event.sequence) ||
              event.sequence <= cursor
            )
              throw new Error(
                "Task activity returned an invalid sequence. Refresh the task to retry.",
              );
            const size = new TextEncoder().encode(
              JSON.stringify(event),
            ).byteLength;
            if (events.length >= MAX_EVENTS || bytes + size > MAX_BYTES) {
              stopped = true;
              setLimited(true);
              break;
            }
            events.push(event);
            bytes += size;
            cursor = event.sequence;
          }
          setUpdates([...events]);
          setError("");
          if (stopped || page.next_after == null) break;
          if (page.next_after !== cursor || !page.updates.length)
            throw new Error(
              "Task activity returned an invalid cursor. Reopen the task to retry.",
            );
        }
      } catch (failure) {
        if (alive) setError(messageOf(failure));
      } finally {
        pending = false;
      }
    }
    wake.current = () => {
      void read();
    };
    const resume = () => {
      void read();
    };
    const timer = setInterval(resume, 5000);
    document.addEventListener("visibilitychange", resume);
    window.addEventListener("focus", resume);
    void read();
    return () => {
      alive = false;
      wake.current = () => {};
      clearInterval(timer);
      document.removeEventListener("visibilitychange", resume);
      window.removeEventListener("focus", resume);
    };
  }, [client, project, task]);
  return { updates, error, limited, refresh: () => wake.current() };
}
