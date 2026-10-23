import { useEffect, useRef, useState } from "react";
import { request } from "./api";
import type { Member, Project } from "./control-api";
import { errorMessage, useResource } from "./resources";

export interface UserProject {
  project: Project;
  membership: Omit<Member, "display_name">;
}
export interface UserWorkspace {
  workspace_id: string;
  project_id: string;
  host_id: string;
  node_id: string;
  label: string;
  sharing: string;
  revision: number;
}
type InventoryPage<T, K extends string> = Record<K, T[]> & {
  next_cursor?: string | null;
  visibility?: "visible" | "personal_resources_private";
};

/** Continuations remain bound to the exact account, path, and first page. */
export function useUserInventory<T, K extends string>(
  path: string | null,
  field: K,
  identity: string,
) {
  const resource = useResource<InventoryPage<T, K>>(path, 0, identity);
  const scope = JSON.stringify([identity, path]);
  const pending = useRef<AbortController | null>(null);
  const [continuation, setContinuation] = useState<{
    scope: string;
    source: InventoryPage<T, K> | null;
    rows: T[];
    cursor: string | null;
    busy: boolean;
    error: string;
  } | null>(null);
  useEffect(
    () => () => {
      pending.current?.abort();
      pending.current = null;
    },
    [scope, resource.data],
  );
  const current =
    continuation?.scope === scope && continuation.source === resource.data
      ? continuation
      : null;
  const rows = [...(resource.data?.[field] ?? []), ...(current?.rows ?? [])];
  const cursor = current
    ? current.cursor
    : (resource.data?.next_cursor ?? null);
  async function loadMore() {
    if (!path || !cursor || !resource.data || pending.current) return;
    const abort = new AbortController();
    pending.current = abort;
    const source = resource.data;
    const previous = current?.rows ?? [];
    setContinuation({
      scope,
      source,
      rows: previous,
      cursor,
      busy: true,
      error: "",
    });
    try {
      const page = await request<InventoryPage<T, K>>(
        `${path}${path.includes("?") ? "&" : "?"}after=${encodeURIComponent(cursor)}`,
        undefined,
        abort.signal,
      );
      if (!abort.signal.aborted)
        setContinuation({
          scope,
          source,
          rows: [...previous, ...page[field]],
          cursor: page.next_cursor ?? null,
          busy: false,
          error: "",
        });
    } catch (error) {
      if (!abort.signal.aborted)
        setContinuation({
          scope,
          source,
          rows: previous,
          cursor,
          busy: false,
          error: errorMessage(error),
        });
    } finally {
      if (pending.current === abort) pending.current = null;
    }
  }
  return {
    ...resource,
    rows,
    cursor,
    moreBusy: current?.busy ?? false,
    moreError: current?.error ?? "",
    visibility: resource.data?.visibility,
    loadMore,
  };
}
export function userApiPath(user: string) {
  return `/api/admin/users/${encodeURIComponent(user)}`;
}
export function userDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}
