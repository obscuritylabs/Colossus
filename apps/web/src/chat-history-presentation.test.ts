import { expect, it } from "vitest";
import type { Thread } from "./api";
import { chatUpdatedTime, groupChatHistory } from "./chat-history-presentation";

const now = new Date(2026, 9, 21, 12);
function thread(id: string, updatedAt?: string): Thread {
  return { thread_id: id, updated_at: updatedAt } as Thread;
}
function localTime(day: number, hour = 12): string {
  return new Date(2026, 9, day, hour).toISOString();
}

it("groups only the released page by local calendar boundaries without sorting it", () => {
  const input = [
    thread("today-first", localTime(21, 8)),
    thread("today-second", localTime(21, 10)),
    thread("yesterday", localTime(20, 23)),
    thread("two-days-ago", localTime(19)),
    thread("seven-days-ago", localTime(14)),
    thread("earlier", localTime(13, 23)),
    thread("invalid", "not a date"),
    thread("missing"),
    thread("future", localTime(22)),
  ];
  const groups = groupChatHistory(input, now);
  expect(
    groups.map((group) => ({
      label: group.label,
      ids: group.threads.map((thread) => thread.thread_id),
    })),
  ).toEqual([
    { label: "Today", ids: ["today-first", "today-second"] },
    { label: "Yesterday", ids: ["yesterday"] },
    { label: "Previous 7 days", ids: ["two-days-ago", "seven-days-ago"] },
    { label: "Earlier", ids: ["earlier"] },
    { label: "Other chats", ids: ["invalid", "missing", "future"] },
  ]);
  expect(groups[0]!.threads[0]).toBe(input[0]);
  expect(input.map((thread) => thread.thread_id)).toEqual([
    "today-first",
    "today-second",
    "yesterday",
    "two-days-ago",
    "seven-days-ago",
    "earlier",
    "invalid",
    "missing",
    "future",
  ]);
});

it("keeps interleaved server order with contiguous groups and omits empty groups", () => {
  const input = [
    thread("earlier", localTime(1)),
    thread("today", localTime(21)),
    thread("unavailable", ""),
    thread("today-again", localTime(21, 11)),
  ];
  const groups = groupChatHistory(input, now);
  expect(groups.map((group) => group.label)).toEqual([
    "Earlier",
    "Today",
    "Other chats",
    "Today",
  ]);
  expect(groups.flatMap((group) => group.threads)).toEqual(input);
  expect(groupChatHistory([], now)).toEqual([]);
});

it("uses calendar days around daylight-saving transitions", () => {
  const autumn = new Date(2026, 10, 2, 0, 30);
  const groups = groupChatHistory(
    [
      thread("today", new Date(2026, 10, 2, 0, 5).toISOString()),
      thread("yesterday", new Date(2026, 10, 1, 0, 5).toISOString()),
      thread("week", new Date(2026, 9, 26, 0, 5).toISOString()),
    ],
    autumn,
  );
  expect(groups.map((group) => group.label)).toEqual([
    "Today",
    "Yesterday",
    "Previous 7 days",
  ]);
});

it("formats recent updated times and retains a complete machine-readable timestamp", () => {
  const value = localTime(21, 9);
  const updated = chatUpdatedTime(value, now);
  expect(updated.label).toMatch(/^Today, /);
  expect(updated.label.length).toBeGreaterThan(7);
  expect(updated.exact).toContain("2026");
  expect(updated.exact.length).toBeGreaterThan(updated.label.length);
  expect(updated.dateTime).toBe(value);
  expect(chatUpdatedTime(localTime(20, 9), now).label).toMatch(/^Yesterday, /);
});

it("includes the year for older calendar years and leaves invalid times unavailable", () => {
  expect(
    chatUpdatedTime(new Date(2025, 11, 31).toISOString(), now).label,
  ).toContain("2025");
  expect(chatUpdatedTime(localTime(1), now).label).not.toContain("2026");
  for (const invalid of ["", "not a date"]) {
    expect(chatUpdatedTime(invalid, now)).toEqual({
      label: "Update time unavailable",
      exact: "Update time unavailable",
    });
  }
});
