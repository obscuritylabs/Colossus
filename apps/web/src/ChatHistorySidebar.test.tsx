// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatHistorySidebar } from "./ChatHistorySidebar";
import { NavigationProvider } from "./navigation";
import type { Thread } from "./api";

let root: Root, container: HTMLDivElement;
const paths: URL[] = [];
const selected = { thread_id: "current" } as Thread;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  paths.length = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string) => {
      const url = new URL(path, "https://control.example");
      paths.push(url);
      const older = url.searchParams.has("after");
      return new Response(
        JSON.stringify({
          threads: [
            {
              thread_id: older ? "older" : "current",
              title: older ? "Earlier conversation" : "Current conversation",
            },
          ],
          next_cursor: older ? null : "server-cursor",
        }),
      );
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(refreshKey = 0) {
  await act(async () =>
    root.render(
      <NavigationProvider>
        <ChatHistorySidebar
          project="p"
          identity="alice"
          selected={selected}
          refreshKey={refreshKey}
        />
      </NavigationProvider>,
    ),
  );
}
function button(text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent?.trim() === text,
  )!;
}

it("retains server cursors, search and selection in conversation navigation", async () => {
  await render();
  expect(container.querySelector('a[aria-current="page"]')?.textContent).toBe(
    "Current conversation",
  );
  expect(paths.at(-1)?.searchParams.get("archived")).toBe("false");
  await act(() => button("Older chats").click());
  expect(paths.at(-1)?.searchParams.get("after")).toBe("server-cursor");
  expect(container.textContent).toContain("Earlier conversation");
  await render(1);
  expect(paths.at(-1)?.searchParams.get("after")).toBe("server-cursor");
  const input = container.querySelector<HTMLInputElement>(
    'input[type="search"]',
  )!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, "migration");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => new Promise((resolve) => setTimeout(resolve, 300)));
  expect(paths.at(-1)?.searchParams.get("query")).toBe("migration");
  expect(paths.at(-1)?.searchParams.has("after")).toBe(false);
  expect(paths.every((url) => !url.searchParams.has("user_id"))).toBe(true);
});
