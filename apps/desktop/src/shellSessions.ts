import { useEffect, useState } from "react";
import { listShellSessions } from "./api";

export type ShellStatus =
  | "starting"
  | "running"
  | "stopping"
  | "exited"
  | "stopped"
  | "timed_out"
  | "failed"
  | "interrupted"
  | "outcome_unknown";
export interface ShellSession {
  id: string;
  session_id: string;
  run_id: string;
  lifetime: "run" | "workspace";
  status: ShellStatus;
  command: string;
  cwd: string;
  created_at_ms: number;
  deadline_ms: number | null;
  exit_code: number | null;
  reason: string | null;
  truncated: boolean;
  output_sequence: number;
}
export interface ShellChunk {
  sequence: number;
  stdout: string;
  stderr: string;
}
export interface ShellSnapshot {
  session: ShellSession;
  chunks: ShellChunk[];
  next_sequence: number;
  gap: boolean;
}
export interface ShellPage {
  sessions: ShellSession[];
  next_cursor: string | null;
}
export function shellIsActive(status: ShellStatus): boolean {
  return status === "starting" || status === "running" || status === "stopping";
}

export function appendShellLog(
  previous: string,
  chunks: ShellChunk[],
): { text: string; truncated: boolean } {
  const text =
    previous + chunks.map((chunk) => chunk.stdout + chunk.stderr).join("");
  return { text: text.slice(-65536), truncated: text.length > 65536 };
}

export function useShellSessions(scope: string | null, fixture: boolean) {
  const [state, setState] = useState<{
    scope: string | null;
    sessions: ShellSession[];
    error: string | null;
  }>({ scope: null, sessions: [], error: null });
  const [revision, refresh] = useState(0);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (!scope || fixture) {
      setState({ scope: null, sessions: [], error: null });
      return;
    }
    async function load() {
      try {
        let after: string | null = null;
        const sessions: ShellSession[] = [];
        for (let pageNumber = 0; pageNumber < 3; pageNumber++) {
          const page: ShellPage = await listShellSessions(scope!, after);
          if (cancelled) return;
          sessions.push(...page.sessions);
          after = page.next_cursor;
          if (!after) break;
          // Page at the public API's default two-requests-per-second rate.
          await new Promise((resolve) => setTimeout(resolve, 550));
          if (cancelled) return;
        }
        setState({ scope, sessions, error: null });
      } catch {
        if (!cancelled)
          setState((previous) => ({
            scope,
            sessions: previous.scope === scope ? previous.sessions : [],
            error:
              "Shell sessions are unavailable on this runtime. Reconnect or update the runtime, then refresh.",
          }));
      }
      if (!cancelled) timer = setTimeout(() => void load(), 5000);
    }
    void load();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [scope, fixture, revision]);
  const current = state.scope === scope ? state : { sessions: [], error: null };
  return {
    ...current,
    loading: Boolean(scope && !fixture && state.scope !== scope),
    refresh: () => refresh((value) => value + 1),
  };
}
