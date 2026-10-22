import type { Thread } from "./api";

function parsedDate(value: string | undefined): Date | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

function calendarDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/** Group the released page without changing its server-supplied order. */
export function groupChatHistory(
  threads: Thread[],
  now = new Date(),
): { label: string; threads: Thread[] }[] {
  const today = calendarDay(now);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const previousWeek = new Date(today);
  previousWeek.setDate(previousWeek.getDate() - 7);
  const groups: { label: string; threads: Thread[] }[] = [];
  for (const thread of threads) {
    const updated = parsedDate(thread.updated_at);
    const day = updated ? calendarDay(updated) : null;
    const label =
      !day || Number.isNaN(today.getTime()) || day > today
        ? "Other chats"
        : day.getTime() === today.getTime()
          ? "Today"
          : day.getTime() === yesterday.getTime()
            ? "Yesterday"
            : day >= previousWeek
              ? "Previous 7 days"
              : "Earlier";
    const group = groups.at(-1);
    if (group?.label === label) group.threads.push(thread);
    else groups.push({ label, threads: [thread] });
  }
  return groups;
}

/** Updated time describes conversation changes, including metadata edits. */
export function chatUpdatedTime(
  value: string,
  now = new Date(),
): { label: string; exact: string; dateTime?: string } {
  const date = parsedDate(value);
  if (!date) {
    return {
      label: "Update time unavailable",
      exact: "Update time unavailable",
    };
  }
  const today = calendarDay(now);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const day = calendarDay(date);
  const clock = date.toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
  const label =
    day.getTime() === today.getTime()
      ? `Today, ${clock}`
      : day.getTime() === yesterday.getTime()
        ? `Yesterday, ${clock}`
        : date.toLocaleDateString([], {
            month: "short",
            day: "numeric",
            ...(date.getFullYear() !== now.getFullYear()
              ? { year: "numeric" }
              : {}),
          });
  return {
    label,
    exact: date.toLocaleString([], {
      dateStyle: "full",
      timeStyle: "long",
    }),
    dateTime: date.toISOString(),
  };
}
