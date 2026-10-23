// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { activeBanner, type AnnouncementEntry } from "./announcements";
import { useAnnouncements } from "./useAnnouncements";

const notice = (id = "notice", publication = 1): AnnouncementEntry => ({
  announcement: {
    id,
    revision: publication,
    publication_revision: publication,
    title: `Update ${id}`,
    summary: "A new instance feature.",
    body: "**Feature details**",
    tone: "info",
    banner: true,
    priority: 50,
    status: "published",
    published_at: "2026-10-22T12:00:00Z",
    expires_at: null,
    created_at: "2026-10-22T12:00:00Z",
    updated_at: "2026-10-22T12:00:00Z",
  },
  read: false,
  dismissed: false,
});
interface Pending {
  path: string;
  options: RequestInit;
  respond: (body: unknown, status?: number) => void;
}
let container: HTMLDivElement, root: Root;
let current: ReturnType<typeof useAnnouncements>;
let calls: Pending[];
function Probe({ identity }: { identity: string }) {
  current = useAnnouncements(identity);
  return (
    <span>
      {current.announcements
        .map((entry) => entry.announcement.title)
        .join(", ")}
    </span>
  );
}
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.spyOn(document, "hidden", "get").mockReturnValue(false);
  calls = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      (path: string, options: RequestInit) =>
        new Promise<Response>((resolve) => {
          calls.push({
            path,
            options,
            respond: (body, status = 200) =>
              resolve(new Response(JSON.stringify(body), { status })),
          });
        }),
    ),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function render(identity = "alice") {
  await act(() => root.render(<Probe identity={identity} />));
}
async function feed(
  call: Pending,
  announcements: AnnouncementEntry[],
  next_cursor: string | null = null,
) {
  await act(() =>
    call.respond({
      announcements,
      next_cursor,
      active_banner: activeBanner(announcements) ?? null,
    }),
  );
}
async function receive(
  call: Pending,
  publication_revision: number,
  dismissed = false,
) {
  await act(() =>
    call.respond({ publication_revision, read: true, dismissed }),
  );
}
async function receipt(entry: AnnouncementEntry, dismissed = false) {
  await act(() => {
    void current.receipt(entry, dismissed);
  });
}
async function revalidate() {
  await act(() =>
    window.dispatchEvent(new Event("colossus:web:announcements-changed")),
  );
}

it("keeps a banner visible until the exact publication dismissal is acknowledged", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!, true);
  const write = calls[1]!;
  expect(write.path).toBe("/api/announcements/notice/receipt");
  expect(JSON.parse(String(write.options.body))).toEqual({
    publication_revision: 1,
    read: true,
    dismissed: true,
  });
  expect(activeBanner(current.announcements)?.announcement.id).toBe("notice");
  expect(current.busyIds.has("notice")).toBe(true);
  await receive(write, 1, true);
  expect(activeBanner(current.announcements)).toBeUndefined();
  expect(current.announcements[0]).toMatchObject({
    read: true,
    dismissed: true,
  });
  expect(current.busyIds.size).toBe(0);
});

it("persists read and dismissal separately and preserves acknowledged receipts across a stale feed", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!);
  expect(JSON.parse(String(calls[1]!.options.body))).toEqual({
    publication_revision: 1,
    read: true,
  });
  await receive(calls[1]!, 1);
  expect(current.announcements[0]).toMatchObject({
    read: true,
    dismissed: false,
  });
  expect(activeBanner(current.announcements)).toBeDefined();
  await receipt(current.announcements[0]!, true);
  await receive(calls[2]!, 1, true);
  await revalidate();
  await feed(calls[3]!, [notice()]);
  expect(current.announcements[0]).toMatchObject({
    read: true,
    dismissed: true,
  });
  expect(activeBanner(current.announcements)).toBeUndefined();
});

it("makes a new publication unread and visible without applying a prior publication receipt", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!, true);
  const priorWrite = calls[1]!;
  await revalidate();
  await feed(calls[2]!, [notice("notice", 2)]);
  await receive(priorWrite, 1, true);
  expect(current.announcements[0]).toMatchObject({
    announcement: { publication_revision: 2 },
    read: false,
    dismissed: false,
  });
  expect(
    activeBanner(current.announcements)?.announcement.publication_revision,
  ).toBe(2);
  await receipt(current.announcements[0]!, true);
  const newPublicationWrite = calls
    .filter((call) => call.options.method === "POST")
    .at(-1)!;
  expect(JSON.parse(String(newPublicationWrite.options.body))).toMatchObject({
    publication_revision: 2,
  });
  await receive(newPublicationWrite, 2, true);
  expect(activeBanner(current.announcements)).toBeUndefined();
});

it("allows different rows in parallel while fencing duplicate actions for the same row", async () => {
  await render();
  await feed(calls[0]!, [notice("one"), notice("two")]);
  await receipt(current.announcements[0]!);
  await receipt(current.announcements[1]!);
  await receipt(current.announcements[0]!, true);
  expect(calls).toHaveLength(3);
  expect([...current.busyIds]).toEqual(["one", "two"]);
  await receive(calls[2]!, 1);
  expect([...current.busyIds]).toEqual(["one"]);
  await receive(calls[1]!, 1);
  expect(current.announcements.every((entry) => entry.read)).toBe(true);
  expect(current.busyIds.size).toBe(0);
});

it("aborts prior-owner requests and ignores delayed feed and receipt responses", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!, true);
  const priorWrite = calls[1]!;
  await revalidate();
  const priorFeed = calls[2]!;
  await render("bob");
  expect((priorWrite.options.signal as AbortSignal).aborted).toBe(true);
  expect((priorFeed.options.signal as AbortSignal).aborted).toBe(true);
  expect(current.announcements).toEqual([]);
  expect(current.busyIds.size).toBe(0);
  await feed(calls[3]!, [notice()]);
  await receive(priorWrite, 1, true);
  await feed(priorFeed, [{ ...notice(), read: true, dismissed: true }]);
  expect(current.announcements[0]).toMatchObject({
    read: false,
    dismissed: false,
  });
  expect(current.error).toBe("");
  await receipt(current.announcements[0]!, true);
  expect(calls[4]!.options.method).toBe("POST");
  await receive(calls[4]!, 1, true);
  expect(current.announcements[0]?.dismissed).toBe(true);
});

it("queues a publication refresh that arrives during an existing feed request", async () => {
  await render();
  await revalidate();
  await revalidate();
  expect(calls).toHaveLength(1);
  await feed(calls[0]!, [notice()]);
  expect(calls).toHaveLength(2);
  await feed(calls[1]!, [notice("notice", 2)]);
  expect(current.announcements[0]?.announcement.publication_revision).toBe(2);
  expect(current.loading).toBe(false);
});

it("does not treat a mismatched acknowledgement as a successful dismissal", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!, true);
  await receive(calls[1]!, 2, true);
  expect(current.announcements[0]).toMatchObject({
    read: false,
    dismissed: false,
  });
  expect(current.receiptError).toContain("changed");
  expect(activeBanner(current.announcements)).toBeDefined();
});

it("refreshes a confirmed receipt conflict without silently retrying the mutation", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!, true);
  await act(() => calls[1]!.respond({ error: "conflict" }, 409));
  expect(calls.map((call) => call.options.method)).toEqual([
    "GET",
    "POST",
    "GET",
  ]);
  await feed(calls[2]!, [notice("notice", 2)]);
  expect(current.announcements[0]).toMatchObject({
    read: false,
    dismissed: false,
  });
  expect(current.busyIds.size).toBe(0);
});

it("keeps a failed dismissal visible and retries only after another explicit action", async () => {
  await render();
  await feed(calls[0]!, [notice()]);
  await receipt(current.announcements[0]!, true);
  await act(() => calls[1]!.respond({ error: "storage" }, 503));
  expect(activeBanner(current.announcements)).toBeDefined();
  expect(current.receiptError).toContain("temporarily unavailable");
  expect(current.busyIds.size).toBe(0);
  expect(calls).toHaveLength(2);
  await receipt(current.announcements[0]!, true);
  expect(calls[2]!.options.body).toBe(calls[1]!.options.body);
  await receive(calls[2]!, 1, true);
  expect(activeBanner(current.announcements)).toBeUndefined();
  expect(current.receiptError).toBe("");
});

it("retains history truncation metadata when the bounded pagination limit is reached", async () => {
  await render();
  await feed(calls[0]!, [notice("page-0")], "cursor-1");
  for (let wantedPages = 2; wantedPages <= 10; wantedPages++) {
    const start = calls.length;
    await act(() => current.loadMore!());
    for (let page = 0; page < wantedPages; page++)
      await feed(
        calls[start + page]!,
        [notice(`page-${page}`)],
        `cursor-${page + 1}`,
      );
  }
  expect(current.announcements).toHaveLength(10);
  expect(current.hasMore).toBe(true);
  expect(current.loadMore).toBeUndefined();
  expect(current.next_cursor).toBe("cursor-10");
});

it("discovers and acknowledges an independent banner outside the loaded history", async () => {
  await render();
  const banner = notice("outside-history");
  await act(() =>
    calls[0]!.respond({
      announcements: [notice("recent")],
      next_cursor: "older",
      active_banner: banner,
    }),
  );
  expect(current.announcements.map((entry) => entry.announcement.id)).toEqual([
    "recent",
  ]);
  expect(current.active_banner?.announcement.id).toBe("outside-history");
  await receipt(current.active_banner!, true);
  await receive(calls[1]!, 1, true);
  expect(current.active_banner?.dismissed).toBe(true);
  expect(calls[2]!.options.method).toBe("GET");
  await act(() =>
    calls[2]!.respond({
      announcements: [notice("recent")],
      next_cursor: "older",
      active_banner: notice("another-banner"),
    }),
  );
  expect(current.active_banner?.announcement.id).toBe("another-banner");
});

it("retains later confirmed receipts when overlapping pages repeat the same content revision", async () => {
  await render();
  await feed(calls[0]!, [notice()], "older");
  await act(() => current.loadMore!());
  await feed(calls[1]!, [notice()], "older");
  const acknowledged = { ...notice(), read: true, dismissed: true };
  await feed(calls[2]!, [acknowledged]);
  expect(current.announcements).toEqual([acknowledged]);
  expect(current.active_banner).toEqual(acknowledged);
  expect(activeBanner(current.announcements)).toBeUndefined();
  expect(calls.every((call) => call.options.method === "GET")).toBe(true);
});
