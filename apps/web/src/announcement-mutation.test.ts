import { expect, it } from "vitest";
import {
  announcementPath,
  announcementWriteMatches,
  type AnnouncementWrite,
} from "./announcement-mutation";
import type { Announcement } from "./announcements";

const write: AnnouncementWrite = {
  id: "stable-id",
  revision: 5,
  action: "publish",
  content: {
    title: "Feature update",
    summary: "A short summary",
    body: "**Exact Markdown**\n",
    tone: "success",
    banner: true,
    priority: 50,
    expires_at: null,
  },
};
const released = (patch: Partial<Announcement> = {}): Announcement => ({
  ...write.content,
  id: write.id,
  revision: 6,
  publication_revision: 2,
  status: "published",
  published_at: "2026-10-22T12:00:00Z",
  created_at: "2026-10-22T10:00:00Z",
  updated_at: "2026-10-22T12:00:00Z",
  ...patch,
});

it("confines an announcement ID to one encoded path segment", () => {
  expect(announcementPath("a/b?scope=other#fragment")).toBe(
    "/api/admin/announcements/a%2Fb%3Fscope%3Dother%23fragment",
  );
});

it("matches only an acknowledged revision of the same reviewed proposal", () => {
  expect(announcementWriteMatches(write, released())).toBe(true);
  const creation: AnnouncementWrite = {
    id: write.id,
    action: write.action,
    content: write.content,
  };
  expect(announcementWriteMatches(creation, released({ revision: 1 }))).toBe(
    true,
  );
  expect(announcementWriteMatches(write, released({ revision: 5 }))).toBe(
    false,
  );
  expect(announcementWriteMatches(write, released({ id: "another-id" }))).toBe(
    false,
  );
});

it.each(["draft", "publish", "archive"] as const)(
  "binds the %s action to its expected persisted status",
  (action) => {
    const status =
      action === "publish"
        ? "published"
        : action === "archive"
          ? "archived"
          : "draft";
    expect(
      announcementWriteMatches({ ...write, action }, released({ status })),
    ).toBe(true);
    expect(
      announcementWriteMatches(
        { ...write, action },
        released({ status: status === "draft" ? "published" : "draft" }),
      ),
    ).toBe(false);
  },
);

it.each([
  { title: "Different title" },
  { summary: "Different summary" },
  { body: "**Exact Markdown**" },
  { tone: "warning" as const },
  { banner: false },
  { priority: 51 },
  { expires_at: "2027-01-01T00:00:00Z" },
])("rejects changed content fields %j", (patch) => {
  expect(announcementWriteMatches(write, released(patch))).toBe(false);
});

it("normalizes labels while preserving exact Markdown whitespace during reconciliation", () => {
  const reviewed = {
    ...write,
    content: {
      ...write.content,
      title: ` ${write.content.title} `,
      summary: `\n${write.content.summary}\t`,
    },
  };
  expect(announcementWriteMatches(reviewed, released())).toBe(true);
  expect(
    announcementWriteMatches(
      reviewed,
      released({ body: `${write.content.body} ` }),
    ),
  ).toBe(false);
});

it("accepts canonical representations of the same expiration instant and rejects a changed instant", () => {
  const reviewed = {
    ...write,
    content: { ...write.content, expires_at: "2027-01-01T10:00:00+02:00" },
  };
  expect(
    announcementWriteMatches(
      reviewed,
      released({ expires_at: "2027-01-01T08:00:00.000Z" }),
    ),
  ).toBe(true);
  expect(
    announcementWriteMatches(
      reviewed,
      released({ expires_at: "2027-01-01T08:00:00.001Z" }),
    ),
  ).toBe(false);
  expect(
    announcementWriteMatches(reviewed, released({ expires_at: null })),
  ).toBe(false);
  expect(
    announcementWriteMatches(reviewed, released({ expires_at: "not-a-date" })),
  ).toBe(false);
});

it("distinguishes expiration changes smaller than one millisecond", () => {
  const reviewed = {
    ...write,
    content: { ...write.content, expires_at: "2027-01-01T08:00:00.123000001Z" },
  };
  expect(
    announcementWriteMatches(
      reviewed,
      released({ expires_at: "2027-01-01T10:00:00.123000001+02:00" }),
    ),
  ).toBe(true);
  expect(
    announcementWriteMatches(
      reviewed,
      released({ expires_at: "2027-01-01T08:00:00.123000002Z" }),
    ),
  ).toBe(false);
  const invalid = {
    ...write,
    content: { ...write.content, expires_at: "not-a-date" },
  };
  expect(
    announcementWriteMatches(invalid, released({ expires_at: "not-a-date" })),
  ).toBe(false);
});
