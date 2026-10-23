import { useCallback, useEffect, useRef, useState } from "react";
import { ApiFailure, request } from "./api";
import { errorMessage } from "./resources";
import {
  mergeAnnouncementEntries,
  reconcileAnnouncementEntries,
  type AnnouncementEntry,
  type AnnouncementFeed,
  type AnnouncementReceipt,
} from "./announcements";

const PAGE_SIZE = 20;
const MAX_RECENT_PAGES = 10;
interface FeedState extends AnnouncementFeed {
  identity: string;
  loading: boolean;
  loadingMore: boolean;
  error: string;
}
const emptyFeed = (identity: string): FeedState => ({
  identity,
  announcements: [],
  active_banner: null,
  next_cursor: null,
  loading: true,
  loadingMore: false,
  error: "",
});

/** The Web host owns authentication, polling, pagination, and receipt authority. */
export function useAnnouncements(identity: string) {
  const [state, setState] = useState<FeedState>(() => emptyFeed(identity));
  const [pages, setPages] = useState(1);
  const [generation, setGeneration] = useState(0);
  const [busyIds, setBusyIds] = useState<ReadonlySet<string>>(() => new Set());
  const [receiptError, setReceiptError] = useState("");
  const alive = useRef(true);
  const owner = useRef(identity);
  const mutations = useRef(new Map<string, AbortController>());
  const refreshing = useRef(false);
  owner.current = identity;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      for (const abort of mutations.current.values()) abort.abort();
      mutations.current.clear();
    };
  }, []);
  useEffect(() => {
    for (const abort of mutations.current.values()) abort.abort();
    mutations.current.clear();
    setBusyIds(new Set());
    setReceiptError("");
    setPages(1);
  }, [identity]);
  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
    let queued = false;
    const load = async () => {
      if (pending) {
        queued = true;
        return;
      }
      pending = true;
      refreshing.current = true;
      setState((old) =>
        old.identity === identity
          ? {
              ...old,
              loading: old.announcements.length === 0,
              loadingMore: pages > 1,
              error: "",
            }
          : emptyFeed(identity),
      );
      try {
        let cursor: string | null = null;
        const entries: AnnouncementEntry[] = [];
        let activeBanner: AnnouncementEntry | null = null;
        const cursors = new Set<string>();
        for (let page = 0; page < Math.min(pages, MAX_RECENT_PAGES); page++) {
          const suffix: string =
            cursor === null ? "" : `&after=${encodeURIComponent(cursor)}`;
          const response: AnnouncementFeed = await request<AnnouncementFeed>(
            `/api/announcements?limit=${PAGE_SIZE}${suffix}`,
            undefined,
            abort.signal,
          );
          entries.push(...response.announcements);
          if (page === 0) activeBanner = response.active_banner;
          cursor = response.next_cursor;
          if (!cursor) break;
          if (cursors.has(cursor))
            throw new Error("Announcement history could not be refreshed.");
          cursors.add(cursor);
        }
        if (!abort.signal.aborted && owner.current === identity) {
          const unique = mergeAnnouncementEntries(entries);
          setState((old) => {
            const previous =
              old.identity === identity
                ? [
                    ...old.announcements,
                    ...(old.active_banner ? [old.active_banner] : []),
                  ]
                : [];
            const announcements = reconcileAnnouncementEntries(
              unique,
              previous,
            );
            const candidate = activeBanner
              ? (reconcileAnnouncementEntries(
                  [activeBanner],
                  [...previous, ...announcements],
                )[0] ?? null)
              : null;
            return {
              identity,
              announcements: candidate
                ? reconcileAnnouncementEntries(announcements, [candidate])
                : announcements,
              active_banner: candidate,
              next_cursor: cursor,
              loading: false,
              loadingMore: false,
              error: "",
            };
          });
        }
      } catch (error) {
        if (!abort.signal.aborted && owner.current === identity)
          setState((old) => ({
            ...(old.identity === identity ? old : emptyFeed(identity)),
            loading: false,
            loadingMore: false,
            error: errorMessage(error),
          }));
      } finally {
        pending = false;
        if (!abort.signal.aborted) refreshing.current = false;
        if (queued && !abort.signal.aborted) {
          queued = false;
          void load();
        }
      }
    };
    void load();
    const interval = window.setInterval(() => {
      if (!document.hidden) void load();
    }, 30_000);
    const revalidate = () => {
      if (!document.hidden) void load();
    };
    window.addEventListener("focus", revalidate);
    window.addEventListener("colossus:web:announcements-changed", revalidate);
    document.addEventListener("visibilitychange", revalidate);
    return () => {
      abort.abort();
      window.clearInterval(interval);
      window.removeEventListener("focus", revalidate);
      window.removeEventListener(
        "colossus:web:announcements-changed",
        revalidate,
      );
      document.removeEventListener("visibilitychange", revalidate);
    };
  }, [identity, pages, generation]);

  const receipt = useCallback(
    async (entry: AnnouncementEntry, dismissed = false) => {
      if (mutations.current.has(entry.announcement.id)) return;
      if (!dismissed && entry.read) return;
      if (mutations.current.size >= 8) {
        setReceiptError(
          "Wait for the current announcement changes, then try again.",
        );
        return;
      }
      const abort = new AbortController();
      const who = identity;
      const publishedRevision = entry.announcement.publication_revision;
      const id = entry.announcement.id;
      mutations.current.set(id, abort);
      setBusyIds(new Set(mutations.current.keys()));
      setReceiptError("");
      try {
        const result = await request<AnnouncementReceipt>(
          `/api/announcements/${encodeURIComponent(id)}/receipt`,
          {
            publication_revision: publishedRevision,
            read: true,
            ...(dismissed ? { dismissed: true } : {}),
          },
          abort.signal,
        );
        if (!abort.signal.aborted && alive.current && owner.current === who) {
          if (result.publication_revision !== publishedRevision)
            throw new Error(
              "This announcement changed. Refresh to read the update.",
            );
          const acknowledge = (item: AnnouncementEntry) =>
            item.announcement.id === id &&
            item.announcement.publication_revision === publishedRevision
              ? {
                  ...item,
                  read: item.read || result.read,
                  dismissed: item.dismissed || result.dismissed,
                }
              : item;
          setState((old) => ({
            ...old,
            announcements: old.announcements.map(acknowledge),
            active_banner: old.active_banner
              ? acknowledge(old.active_banner)
              : null,
          }));
          // The next eligible banner may be outside the bounded history page.
          if (result.dismissed) setGeneration((old) => old + 1);
        }
      } catch (error) {
        if (!abort.signal.aborted && alive.current && owner.current === who) {
          setReceiptError(errorMessage(error));
          if (error instanceof ApiFailure && error.status === 409)
            setGeneration((old) => old + 1);
        }
      } finally {
        if (mutations.current.get(id) === abort) mutations.current.delete(id);
        if (alive.current && owner.current === who)
          setBusyIds(new Set(mutations.current.keys()));
      }
    },
    [identity],
  );
  const refresh = useCallback(() => {
    setReceiptError("");
    setGeneration((old) => old + 1);
  }, []);
  const loadMore = useCallback(() => {
    if (!refreshing.current)
      setPages((old) => Math.min(old + 1, MAX_RECENT_PAGES));
  }, []);
  const visible = state.identity === identity ? state : emptyFeed(identity);
  return {
    ...visible,
    busyIds,
    receiptError,
    error: receiptError || visible.error,
    hasMore: Boolean(visible.next_cursor),
    refresh,
    loadMore: pages < MAX_RECENT_PAGES ? loadMore : undefined,
    receipt,
  };
}
