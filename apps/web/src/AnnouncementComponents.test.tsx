// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  AnnouncementBanner,
  AnnouncementCenter,
  type AnnouncementItem,
} from "@colossus/ui/components/Announcements";
import { WebLink } from "./WebLink";

let container: HTMLDivElement, root: Root;
const current: AnnouncementItem = {
  id: "new-tools",
  publicationRevision: 3,
  title: "New tools are available",
  summary: "A short summary of the update.",
  body: "**A new feature** with [documentation](https://example.com/docs).",
  tone: "info",
  publishedAt: "2026-10-22T13:00:00Z",
  read: false,
  dismissed: false,
};
const dismissed: AnnouncementItem = {
  ...current,
  id: "older-update",
  title: "An earlier update",
  read: true,
  dismissed: true,
};

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});
const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) =>
      item.getAttribute("aria-label") === label ||
      item.textContent?.trim() === label,
  )!;

it("leaves banner persistence to explicit host actions and prevents duplicate busy actions", async () => {
  const read = vi.fn(),
    dismiss = vi.fn();
  const render = (busy = false) =>
    root.render(
      <AnnouncementBanner
        announcement={current}
        busy={busy}
        onRead={read}
        onDismiss={dismiss}
      />,
    );
  await act(() => render());
  expect(read).not.toHaveBeenCalled();
  expect(dismiss).not.toHaveBeenCalled();
  expect(
    container.querySelector('[aria-label="Instance announcement"]')
      ?.textContent,
  ).toContain(current.summary);
  await act(() => button("Read update").click());
  await act(() => button(`Dismiss announcement: ${current.title}`).click());
  expect(read).toHaveBeenCalledTimes(1);
  expect(dismiss).toHaveBeenCalledTimes(1);
  await act(() => render(true));
  expect(button("Read update").disabled).toBe(true);
  expect(button(`Dismiss announcement: ${current.title}`).disabled).toBe(true);
  await act(() => button(`Dismiss announcement: ${current.title}`).click());
  expect(dismiss).toHaveBeenCalledTimes(1);
});

it("labels partial unread counts honestly and opens the controlled center without implicit read receipts", async () => {
  const open = vi.fn(),
    select = vi.fn();
  await act(() =>
    root.render(
      <AnnouncementCenter
        announcements={[current, dismissed]}
        open={false}
        onOpenChange={open}
        onSelect={select}
        hasMore
      />,
    ),
  );
  const bell = button("Announcements, At least 1 unread update");
  expect(bell.textContent).toBe("1+");
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  await act(() => bell.click());
  expect(open).toHaveBeenCalledWith(true);
  expect(select).not.toHaveBeenCalled();
});

it("keeps dismissed announcements readable, exposes only supported actions, and loads older history explicitly", async () => {
  const select = vi.fn(),
    dismiss = vi.fn(),
    more = vi.fn();
  await act(() =>
    root.render(
      <AnnouncementCenter
        announcements={[current, dismissed]}
        open
        onOpenChange={() => {}}
        onSelect={select}
        onDismiss={dismiss}
        hasMore
        onLoadMore={more}
      />,
    ),
  );
  expect(
    document.querySelector('[role="dialog"]')?.getAttribute("aria-labelledby"),
  ).toBeTruthy();
  expect(document.querySelector(".announcement-list")?.textContent).toContain(
    "Banner dismissed",
  );
  expect(
    button(`Show announcement banner again: ${dismissed.title}`),
  ).toBeUndefined();
  expect(select).not.toHaveBeenCalled();
  expect(dismiss).not.toHaveBeenCalled();
  await act(() => button(`Read announcement: ${dismissed.title}`).click());
  expect(select).toHaveBeenCalledWith(dismissed);
  await act(() => button("Load older announcements").click());
  expect(more).toHaveBeenCalledTimes(1);
  await act(() => button(`Dismiss announcement: ${current.title}`).click());
  expect(dismiss).toHaveBeenCalledWith(current);
});

it("shows sanitized Markdown through the host link policy without selecting or marking an update implicitly", async () => {
  const select = vi.fn(),
    back = vi.fn();
  await act(() =>
    root.render(
      <AnnouncementCenter
        announcements={[
          {
            ...current,
            publishedAt: "not-a-date",
            body: `${current.body}\n\n[unsafe](javascript:alert(1)) ![remote](https://example.com/tracker.png)\n\n<script>alert('unsafe')</script>`,
          },
        ]}
        open
        onOpenChange={() => {}}
        selectedId={current.id}
        onSelect={select}
        onBack={back}
        linkComponent={WebLink}
      />,
    ),
  );
  for (
    let attempt = 0;
    attempt < 100 && !document.querySelector(".announcement-detail strong");
    attempt++
  ) {
    await act(async () => new Promise((resolve) => setTimeout(resolve, 20)));
  }
  const detail = document.querySelector(".announcement-detail")!;
  expect(detail.querySelector("strong")?.textContent).toBe("A new feature");
  expect(detail.textContent).toContain("Publication time unavailable");
  const link = detail.querySelector<HTMLAnchorElement>("a")!;
  expect(link.href).toBe("https://example.com/docs");
  expect(link.target).toBe("_blank");
  expect(link.rel).toContain("noopener");
  expect(detail.querySelector("script,img")).toBeNull();
  expect(detail.querySelector('a[href^="javascript:"]')).toBeNull();
  expect(document.querySelector(".announcement-list")).toBeNull();
  expect(select).not.toHaveBeenCalled();
  await act(() => button("All announcements").click());
  expect(back).toHaveBeenCalledTimes(1);
});

it("preserves retry and busy-row behavior without hiding previously received announcements", async () => {
  const retry = vi.fn(),
    select = vi.fn(),
    dismiss = vi.fn();
  await act(() =>
    root.render(
      <AnnouncementCenter
        announcements={[current]}
        open
        onOpenChange={() => {}}
        onSelect={select}
        onDismiss={dismiss}
        busyId={current.id}
        error="Announcements could not be refreshed."
        onRetry={retry}
      />,
    ),
  );
  expect(document.querySelector('[role="alert"]')?.textContent).toContain(
    "could not be refreshed",
  );
  expect(button(`Read announcement: ${current.title}, unread`).disabled).toBe(
    true,
  );
  expect(button(`Dismiss announcement: ${current.title}`).disabled).toBe(true);
  await act(() => button("Try again").click());
  expect(retry).toHaveBeenCalledTimes(1);
  expect(select).not.toHaveBeenCalled();
  expect(dismiss).not.toHaveBeenCalled();
});

it("closes with Escape and restores focus to the bell", async () => {
  function Center() {
    const [open, setOpen] = useState(false);
    return (
      <AnnouncementCenter
        announcements={[]}
        open={open}
        onOpenChange={setOpen}
        onSelect={() => {}}
      />
    );
  }
  await act(() => root.render(<Center />));
  const bell = button("Announcements, 0 unread updates");
  bell.focus();
  await act(() => bell.click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain(
    "No announcements yet.",
  );
  await act(() =>
    document.activeElement!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    ),
  );
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  // Radix restores focus after its unmount cleanup has released the focus scope.
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(document.activeElement).toBe(bell);
});

it("moves keyboard focus into an update and back to the original history row", async () => {
  function Center() {
    const [selectedId, setSelectedId] = useState<string | null>(null);
    return (
      <AnnouncementCenter
        announcements={[current, dismissed]}
        open
        onOpenChange={() => {}}
        selectedId={selectedId}
        onSelect={(item) => setSelectedId(item.id)}
        onBack={() => setSelectedId(null)}
      />
    );
  }
  await act(() => root.render(<Center />));
  const rowLabel = `Read announcement: ${current.title}, unread`;
  button(rowLabel).focus();
  await act(() => button(rowLabel).click());
  expect(document.activeElement).toBe(
    document.querySelector(".announcement-detail h3"),
  );
  await act(() => button("All announcements").click());
  expect(document.activeElement).toBe(button(rowLabel));
});

it("keeps parallel pending receipts scoped to their own rows", async () => {
  const available = { ...current, id: "available", title: "Another update" };
  await act(() =>
    root.render(
      <AnnouncementCenter
        announcements={[current, dismissed, available]}
        open
        onOpenChange={() => {}}
        onSelect={() => {}}
        onDismiss={() => {}}
        busyId={current.id}
        busyIds={new Set([dismissed.id])}
      />,
    ),
  );
  expect(button(`Read announcement: ${current.title}, unread`).disabled).toBe(
    true,
  );
  expect(button(`Read announcement: ${dismissed.title}`).disabled).toBe(true);
  expect(button(`Read announcement: ${available.title}, unread`).disabled).toBe(
    false,
  );
  expect(button(`Dismiss announcement: ${available.title}`).disabled).toBe(
    false,
  );
});

it("returns focus to the center heading when a selected update is no longer available", async () => {
  const render = (announcements: AnnouncementItem[]) =>
    root.render(
      <AnnouncementCenter
        announcements={announcements}
        open
        onOpenChange={() => {}}
        selectedId={current.id}
        onSelect={() => {}}
        onBack={() => {}}
      />,
    );
  await act(() => render([current]));
  await act(() => render([]));
  expect(document.querySelector(".announcement-detail")).toBeNull();
  expect(document.activeElement).toBe(
    document.querySelector(".announcement-center-title"),
  );
});
