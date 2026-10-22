import { request } from "./api";
import {
  announcementContent,
  announcementInstant,
  type Announcement,
  type AnnouncementAction,
  type AnnouncementContent,
} from "./announcements";

export interface AnnouncementWrite {
  id: string;
  revision?: number;
  action: AnnouncementAction;
  content: AnnouncementContent;
}
export const announcementPath = (id: string) =>
  `/api/admin/announcements/${encodeURIComponent(id)}`;

export async function saveAnnouncement(
  write: AnnouncementWrite,
  signal?: AbortSignal,
): Promise<Announcement> {
  const result = await request<{ announcement: Announcement }>(
    write.revision === undefined
      ? "/api/admin/announcements"
      : announcementPath(write.id),
    {
      ...announcementContent(write.content),
      action: write.action,
      ...(write.revision === undefined
        ? { id: write.id }
        : { revision: write.revision }),
    },
    signal,
    write.revision === undefined ? undefined : "PATCH",
  );
  return result.announcement;
}

/** Reconcile an uncertain mutation without issuing another publication. */
export function announcementWriteMatches(
  write: AnnouncementWrite,
  current: Announcement,
): boolean {
  const expected = announcementContent(write.content);
  const actual = announcementContent(current);
  return (
    current.id === write.id &&
    current.revision >= (write.revision ?? 0) + 1 &&
    current.status ===
      (write.action === "publish"
        ? "published"
        : write.action === "archive"
          ? "archived"
          : "draft") &&
    expected.title === actual.title &&
    expected.summary === actual.summary &&
    expected.body === actual.body &&
    expected.tone === actual.tone &&
    expected.banner === actual.banner &&
    expected.priority === actual.priority &&
    sameExpiration(expected.expires_at, actual.expires_at)
  );
}

function sameExpiration(
  expected: string | null,
  actual: string | null,
): boolean {
  if (expected === null || actual === null) return expected === actual;
  const instant = announcementInstant(expected);
  return instant !== null && instant === announcementInstant(actual);
}
