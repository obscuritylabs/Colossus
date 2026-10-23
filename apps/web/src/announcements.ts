export type AnnouncementTone = "info" | "success" | "warning" | "danger";
export type AnnouncementAction = "draft" | "publish" | "archive";

export interface AnnouncementContent {
  title: string;
  summary: string;
  body: string;
  tone: AnnouncementTone;
  banner: boolean;
  priority: number;
  expires_at: string | null;
}

export interface Announcement extends AnnouncementContent {
  id: string;
  revision: number;
  publication_revision: number;
  status: "draft" | "published" | "archived";
  published_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface AnnouncementEntry {
  announcement: Announcement;
  read: boolean;
  dismissed: boolean;
}

export interface AnnouncementFeed {
  announcements: AnnouncementEntry[];
  next_cursor: string | null;
  active_banner: AnnouncementEntry | null;
}

export interface AnnouncementReceipt {
  publication_revision: number;
  read: boolean;
  dismissed: boolean;
}

export const ANNOUNCEMENT_BODY_BYTES = 16_384;

export function announcementContent(
  value: AnnouncementContent,
): AnnouncementContent {
  return {
    title: value.title.trim(),
    summary: value.summary.trim(),
    body: value.body,
    tone: value.tone,
    banner: value.banner,
    priority: value.priority,
    expires_at: value.expires_at,
  };
}

export function announcementValidation(
  value: AnnouncementContent,
  publish = false,
): string {
  if (!value.title.trim() || [...value.title.trim()].length > 120)
    return "Enter a title of up to 120 characters.";
  if ([...value.summary.trim()].length > 240)
    return "Keep the banner summary within 240 characters.";
  if (
    !value.body.trim() ||
    new TextEncoder().encode(value.body).length > ANNOUNCEMENT_BODY_BYTES
  )
    return "Enter Markdown content of up to 16 KiB.";
  if (
    !Number.isInteger(value.priority) ||
    value.priority < 0 ||
    value.priority > 100
  )
    return "Choose a priority between 0 and 100.";
  if (value.expires_at !== null) {
    const time = Date.parse(value.expires_at);
    if (!Number.isFinite(time) || (publish && time <= Date.now()))
      return "Choose an expiration in the future, or leave it empty.";
  }
  return "";
}

function mergeEntry(
  old: AnnouncementEntry,
  entry: AnnouncementEntry,
): AnnouncementEntry {
  if (
    old.announcement.publication_revision >
    entry.announcement.publication_revision
  )
    return old;
  if (
    old.announcement.publication_revision !==
    entry.announcement.publication_revision
  )
    return entry;
  const newest =
    old.announcement.revision > entry.announcement.revision ? old : entry;
  return {
    ...newest,
    read: entry.read || old.read,
    dismissed: entry.dismissed || old.dismissed,
  };
}

/** Deduplicate candidate/history records without losing newer content or receipts. */
export function mergeAnnouncementEntries(
  entries: readonly AnnouncementEntry[],
): AnnouncementEntry[] {
  const byId = new Map<string, AnnouncementEntry>();
  for (const entry of entries) {
    const previous = byId.get(entry.announcement.id);
    byId.set(
      entry.announcement.id,
      previous ? mergeEntry(previous, entry) : entry,
    );
  }
  return [...byId.values()];
}

/** Missing rows leave the feed; known receipts only move forwards within a publication. */
export function reconcileAnnouncementEntries(
  incoming: AnnouncementEntry[],
  previous: AnnouncementEntry[],
): AnnouncementEntry[] {
  const prior = new Map(
    mergeAnnouncementEntries(previous).map((entry) => [
      entry.announcement.id,
      entry,
    ]),
  );
  return mergeAnnouncementEntries(incoming).map((entry) => {
    const old = prior.get(entry.announcement.id);
    return old ? mergeEntry(old, entry) : entry;
  });
}

/** Compare released RFC3339 timestamps without losing Rust's nanoseconds. */
export function announcementInstant(value: string): bigint | null {
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds)) return null;
  // Rust's RFC3339 output can retain nanoseconds. Preserve the remainder that
  // Date.parse truncates so publications within one millisecond stay ordered.
  const fraction = value.match(/\.(\d{1,9})(?:Z|[+-]\d{2}:\d{2})$/u)?.[1] ?? "";
  return (
    BigInt(milliseconds) * 1_000_000n + BigInt(fraction.padEnd(9, "0").slice(3))
  );
}

function publicationInstant(announcement: Announcement): bigint {
  return (
    announcementInstant(announcement.published_at ?? announcement.created_at) ??
    0n
  );
}

export function activeBanner(
  entries: AnnouncementEntry[],
  now = Date.now(),
): AnnouncementEntry | undefined {
  return entries
    .filter(
      ({ announcement, dismissed }) =>
        !dismissed &&
        announcement.banner &&
        announcement.status === "published" &&
        (announcement.expires_at === null ||
          Date.parse(announcement.expires_at) > now),
    )
    .sort(
      (a, b) =>
        b.announcement.priority - a.announcement.priority ||
        Number(
          publicationInstant(b.announcement) -
            publicationInstant(a.announcement),
        ) ||
        a.announcement.id.localeCompare(b.announcement.id),
    )[0];
}
