import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getWorkspaceGitStatus,
  listWorkspaceGitCommits,
  getWorkspaceGitCommit,
} from "../../api";
import type { GitStatus, GitCommitPage, GitCommitDetails } from "../../git";

function message(error: unknown): string {
  return error instanceof Error
    ? error.message
    : typeof error === "object" && error !== null && "message" in error
      ? String(error.message)
      : "Git could not refresh. Try again.";
}

export function useGit(
  workspaceId: string | null,
  available: boolean,
  visible: boolean,
  fixture: boolean,
  refreshKey: string,
) {
  const scope = available ? workspaceId : null;
  // The native reader is shared across workspaces. Keep its queue alive when
  // the selected workspace (and the client that binds it) changes.
  const tail = useRef<Promise<unknown>>(Promise.resolve());
  const client = useMemo(() => {
    const queue = <T>(action: () => Promise<T>): Promise<T> => {
      const result = tail.current.then(action, action);
      tail.current = result.catch(() => undefined);
      return result;
    };
    return {
      status: (approve = false) =>
        queue(async () => {
          if (import.meta.env.DEV && fixture)
            return (await import("../../dev/git-fixture")).gitFixture.status(
              scope!,
            );
          return getWorkspaceGitStatus(scope!, approve);
        }),
      history: (
        repositoryId: string,
        cursor: string | null,
      ): Promise<GitCommitPage> =>
        queue(async () => {
          if (import.meta.env.DEV && fixture)
            return (await import("../../dev/git-fixture")).gitFixture.history(
              cursor,
            );
          return listWorkspaceGitCommits(scope!, repositoryId, cursor);
        }),
      details: (
        repositoryId: string,
        commitId: string,
      ): Promise<GitCommitDetails> =>
        queue(async () => {
          if (import.meta.env.DEV && fixture)
            return (await import("../../dev/git-fixture")).gitFixture.details(
              commitId,
            );
          return getWorkspaceGitCommit(scope!, repositoryId, commitId);
        }),
    };
  }, [scope, fixture]);
  const [result, setResult] = useState<{
    client: typeof client;
    status: GitStatus | null;
    busy: boolean;
    error: string;
  }>({ client, status: null, busy: false, error: "" });
  const generation = useRef(0);
  const inFlight = useRef(false);
  const pending = useRef(false);
  const refresh = useCallback(
    async (approve = false) => {
      if (scope === null) return;
      if (inFlight.current) {
        pending.current = true;
        return;
      }
      inFlight.current = true;
      const version = generation.current;
      setResult((old) => ({
        client,
        status: old.client === client ? old.status : null,
        busy: true,
        error: "",
      }));
      try {
        do {
          pending.current = false;
          const status = await client.status(approve);
          if (version !== generation.current) return;
          setResult({ client, status, busy: true, error: "" });
          approve = false;
        } while (pending.current);
      } catch (error) {
        if (version === generation.current)
          setResult((old) => ({ ...old, error: message(error) }));
      } finally {
        if (version === generation.current) {
          inFlight.current = false;
          setResult((old) => ({ ...old, busy: false }));
        }
      }
    },
    [client, scope],
  );
  useEffect(() => {
    generation.current += 1;
    inFlight.current = false;
    pending.current = false;
    return () => {
      generation.current += 1;
    };
  }, [refresh]);
  useEffect(() => {
    if (scope === null) return;
    const update = () => {
      if (!document.hidden) void refresh();
    };
    window.addEventListener("focus", update);
    document.addEventListener("visibilitychange", update);
    const timer = window.setInterval(update, visible ? 10_000 : 30_000);
    return () => {
      window.removeEventListener("focus", update);
      document.removeEventListener("visibilitychange", update);
      window.clearInterval(timer);
    };
  }, [scope, visible, refresh]);
  useEffect(() => {
    void refresh();
  }, [refreshKey, refresh]);
  return {
    status: result.client === client ? result.status : null,
    busy: result.client === client && result.busy,
    error: result.client === client ? result.error : "",
    refresh,
    client,
    available: scope !== null,
  };
}
export type GitController = ReturnType<typeof useGit>;
