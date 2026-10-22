import { afterEach, expect, it, vi } from "vitest";
import {
  ANNOUNCEMENT_BODY_BYTES,
  activeBanner,
  announcementContent,
  announcementValidation,
  mergeAnnouncementEntries,
  reconcileAnnouncementEntries,
  type AnnouncementContent,
  type AnnouncementEntry,
} from "./announcements";

const content: AnnouncementContent = {
  title: "Feature update",
  summary: "A compact summary.",
  body: "**Feature details**\n",
  tone: "info",
  banner: true,
  priority: 50,
  expires_at: null,
};
const now = Date.parse("2026-10-22T12:00:00Z");
const entry = (
  id: string,
  patch: Partial<AnnouncementEntry["announcement"]> = {},
): AnnouncementEntry => ({
  announcement: {
    ...content,
    id,
    revision: 1,
    publication_revision: 1,
    status: "published",
    published_at: "2026-10-22T10:00:00Z",
    created_at: "2026-10-22T09:00:00Z",
    updated_at: "2026-10-22T10:00:00Z",
    ...patch,
  },
  read: false,
  dismissed: false,
});
afterEach(() => vi.restoreAllMocks());

it("normalizes labels while preserving exact Markdown bytes and leaving the proposal untouched", () => {
  const proposal = {
    ...content,
    title: " \tFeature update\n",
    summary: "  A compact summary.\n",
    body: " \n```text\n  preserve this\n```\n ",
  };
  const original = structuredClone(proposal);
  expect(announcementContent(proposal)).toEqual({
    ...original,
    title: "Feature update",
    summary: "A compact summary.",
  });
  expect(proposal).toEqual(original);
});

it("accepts scalar character limits for Unicode titles and summaries", () => {
  expect(
    announcementValidation({
      ...content,
      title: `  ${"🙂".repeat(120)}  `,
      summary: "界".repeat(240),
    }),
  ).toBe("");
  expect(
    announcementValidation({ ...content, title: "🙂".repeat(121) }),
  ).toContain("120 characters");
  expect(
    announcementValidation({ ...content, summary: "🙂".repeat(241) }),
  ).toContain("240 characters");
  expect(announcementValidation({ ...content, title: " \n\t " })).toContain(
    "Enter a title",
  );
});

it("enforces Markdown bounds by UTF-8 bytes without splitting valid characters", () => {
  expect(
    announcementValidation({
      ...content,
      body: "a".repeat(ANNOUNCEMENT_BODY_BYTES),
    }),
  ).toBe("");
  expect(
    announcementValidation({
      ...content,
      body: "a".repeat(ANNOUNCEMENT_BODY_BYTES + 1),
    }),
  ).toContain("16 KiB");
  const unicode = "🙂".repeat(ANNOUNCEMENT_BODY_BYTES / 4);
  expect(new TextEncoder().encode(unicode).length).toBe(
    ANNOUNCEMENT_BODY_BYTES,
  );
  expect(announcementValidation({ ...content, body: unicode })).toBe("");
  expect(announcementValidation({ ...content, body: `${unicode}a` })).toContain(
    "16 KiB",
  );
  expect(announcementValidation({ ...content, body: " \n\t " })).toContain(
    "Markdown content",
  );
});

it.each([-1, 101, 0.5, Number.NaN, Number.POSITIVE_INFINITY])(
  "rejects invalid priority %s",
  (priority) => {
    expect(announcementValidation({ ...content, priority })).toContain(
      "between 0 and 100",
    );
  },
);

it("allows expired drafts but requires a future expiration for publication", () => {
  vi.spyOn(Date, "now").mockReturnValue(now);
  expect(announcementValidation({ ...content, priority: 0 })).toBe("");
  expect(announcementValidation({ ...content, priority: 100 })).toBe("");
  expect(
    announcementValidation({ ...content, expires_at: "not-a-date" }),
  ).toContain("expiration");
  expect(
    announcementValidation({
      ...content,
      expires_at: new Date(now).toISOString(),
    }),
  ).toBe("");
  expect(
    announcementValidation(
      { ...content, expires_at: new Date(now).toISOString() },
      true,
    ),
  ).toContain("future");
  expect(
    announcementValidation(
      { ...content, expires_at: new Date(now + 1).toISOString() },
      true,
    ),
  ).toBe("");
});

it("selects an active banner by priority, recency, and stable ID without sorting the supplied feed", () => {
  const values = [
    entry("low", { priority: 0 }),
    entry("older", { priority: 100 }),
    entry("b", { priority: 100, published_at: "2026-10-22T11:00:00Z" }),
    entry("a", { priority: 100, published_at: "2026-10-22T11:00:00Z" }),
  ];
  const order = values.map(({ announcement }) => announcement.id);
  expect(activeBanner(values, now)?.announcement.id).toBe("a");
  expect(values.map(({ announcement }) => announcement.id)).toEqual(order);
});

it("omits dismissed, expired, archived, draft, and notification-only banners while retaining read banners", () => {
  const values = [
    { ...entry("dismissed", { priority: 100 }), dismissed: true },
    entry("expired", {
      priority: 100,
      expires_at: new Date(now).toISOString(),
    }),
    entry("invalid-expiry", { priority: 100, expires_at: "invalid" }),
    entry("archived", { priority: 100, status: "archived" }),
    entry("draft", { priority: 100, status: "draft" }),
    entry("notification", { priority: 100, banner: false }),
    { ...entry("read"), read: true },
  ];
  expect(activeBanner(values, now)?.announcement.id).toBe("read");
  expect(activeBanner(values.slice(0, -1), now)).toBeUndefined();
  expect(
    activeBanner([entry("legacy-date", { published_at: null })], now)
      ?.announcement.id,
  ).toBe("legacy-date");
});

it("compares publication instants across RFC3339 precision and timezone spellings", () => {
  const older = entry("a", { published_at: "2026-10-22T11:00:00.123Z" });
  const newer = entry("b", { published_at: "2026-10-22T11:00:00.123001Z" });
  expect(activeBanner([older, newer], now)?.announcement.id).toBe("b");
  const offset = entry("a", { published_at: "2026-10-22T12:00:00+02:00" });
  const utc = entry("b", { published_at: "2026-10-22T10:30:00Z" });
  expect(activeBanner([offset, utc], now)?.announcement.id).toBe("b");
});

it("keeps the newest content and acknowledged flags within one publication", () => {
  const prior = {
    ...entry("same", { revision: 3, title: "Current content" }),
    read: true,
  };
  const stale = {
    ...entry("same", { revision: 2, title: "Old content" }),
    dismissed: true,
  };
  expect(reconcileAnnouncementEntries([stale], [prior])).toEqual([
    { ...prior, dismissed: true },
  ]);
  const incoming = entry("same", { revision: 4, title: "Latest content" });
  expect(reconcileAnnouncementEntries([incoming], [prior])).toEqual([
    { ...incoming, read: true },
  ]);
});

it("resets receipts for a new publication and prevents a lower publication from restoring stale content", () => {
  const previous = { ...entry("same"), read: true, dismissed: true };
  const next = entry("same", {
    revision: 2,
    publication_revision: 2,
    title: "New publication",
  });
  expect(reconcileAnnouncementEntries([next], [previous])).toEqual([next]);
  expect(reconcileAnnouncementEntries([previous], [next])).toEqual([next]);
  expect(reconcileAnnouncementEntries([], [previous])).toEqual([]);
  expect(reconcileAnnouncementEntries([entry("other")], [previous])).toEqual([
    entry("other"),
  ]);
});

it("merges mixed candidate/history revisions and combines receipts only within the newest publication", () => {
  const oldPublication = {
    ...entry("same", { revision: 2, publication_revision: 1 }),
    read: true,
    dismissed: true,
  };
  const current = entry("same", {
    revision: 3,
    publication_revision: 2,
    title: "Current publication",
  });
  const currentRead = { ...current, read: true };
  const edited = {
    ...entry("same", {
      revision: 4,
      publication_revision: 2,
      title: "Current edited content",
    }),
    dismissed: true,
  };
  expect(mergeAnnouncementEntries([current, oldPublication])).toEqual([
    current,
  ]);
  expect(mergeAnnouncementEntries([oldPublication, current])).toEqual([
    current,
  ]);
  const merged = { ...edited, read: true };
  expect(
    mergeAnnouncementEntries([edited, currentRead, oldPublication]),
  ).toEqual([merged]);
  expect(
    mergeAnnouncementEntries([oldPublication, currentRead, edited]),
  ).toEqual([merged]);
  expect(
    reconcileAnnouncementEntries(
      [current],
      [edited, currentRead, oldPublication],
    ),
  ).toEqual([merged]);
  expect(
    reconcileAnnouncementEntries(
      [current, edited, oldPublication],
      [currentRead],
    ),
  ).toEqual([merged]);
});
