// @vitest-environment happy-dom
import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  boundedConversationQuote,
  ConversationSelectionLauncher,
  MAX_CONVERSATION_QUOTE_BYTES,
} from "@colossus/ui/conversation";

interface Source {
  messageId: string;
  taskId: string;
}

interface FixtureProps {
  scopeKey?: string;
  enabled?: boolean;
  resolveSource?: (body: HTMLElement) => Source | null;
  onOpen: (selection: { source: Source; quote: string }) => void;
}

function sourceFor(body: HTMLElement): Source | null {
  const messageId = body.dataset.messageId;
  return messageId ? { messageId, taskId: "released-task" } : null;
}

function Fixture({
  scopeKey = "thread-one",
  enabled = true,
  resolveSource = sourceFor,
  onOpen,
}: FixtureProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  return (
    <>
      <input aria-label="Original composer" />
      <div ref={containerRef}>
        <div data-aside-selectable="true" data-message-id="message-one">
          Released answer with evidence.
        </div>
        <div data-aside-selectable="true" data-message-id="message-two">
          Another released answer.
        </div>
        <div data-message-id="unreleased">Unreleased content.</div>
      </div>
      <div data-aside-selectable="true" data-message-id="outside">
        A separate inspector.
      </div>
      <ConversationSelectionLauncher
        containerRef={containerRef}
        scopeKey={scopeKey}
        enabled={enabled}
        resolveSource={resolveSource}
        onOpen={onOpen}
      />
    </>
  );
}

let container: HTMLDivElement;
let root: Root;
let nativeSelection: Selection | null;
let frames: Map<number, FrameRequestCallback>;
let nextFrame: number;
const removeAllRanges = vi.fn();

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  nativeSelection = null;
  frames = new Map();
  nextFrame = 0;
  removeAllRanges.mockClear();
  const createRange = document.createRange.bind(document);
  vi.spyOn(document, "createRange").mockImplementation(() => {
    const range = createRange();
    range.getBoundingClientRect = () => new DOMRect(120, 180, 130, 22);
    return range;
  });
  vi.spyOn(window, "getSelection").mockImplementation(() => nativeSelection);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    const id = ++nextFrame;
    frames.set(id, callback);
    return id;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => {
    frames.delete(id);
  });
});

afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function render(props: Partial<FixtureProps> = {}) {
  const onOpen = props.onOpen ?? vi.fn();
  await act(() => root.render(<Fixture {...props} onOpen={onOpen} />));
  return onOpen;
}

function body(id = "message-one") {
  return container.querySelector<HTMLElement>(`[data-message-id="${id}"]`)!;
}

function select(
  start: HTMLElement,
  end = start,
  text = "Released answer",
  offset = 0,
) {
  const range = document.createRange();
  range.setStart(start.firstChild!, offset);
  range.setEnd(end.firstChild!, Math.min(offset + 8, end.textContent!.length));
  range.getBoundingClientRect = () => new DOMRect(120, 180, 130, 22);
  nativeSelection = {
    anchorNode: range.startContainer,
    focusNode: range.endContainer,
    rangeCount: 1,
    isCollapsed: false,
    getRangeAt: () => range,
    toString: () => text,
    removeAllRanges,
  } as unknown as Selection;
}

async function selectionChanged() {
  await act(() => {
    document.dispatchEvent(new Event("selectionchange"));
    const pending = [...frames.values()];
    frames.clear();
    for (const callback of pending) callback(0);
  });
}

function launcher() {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Ask in side chat",
  );
}

it("bounds quotes by UTF-8 bytes without partial characters", () => {
  expect(boundedConversationQuote("  a quote \n")).toBe("a quote");
  expect(boundedConversationQuote("a".repeat(5000))).toHaveLength(
    MAX_CONVERSATION_QUOTE_BYTES,
  );
  const emoji = boundedConversationQuote("😀".repeat(2000));
  expect(emoji).toBe("😀".repeat(1024));
  expect(new TextEncoder().encode(emoji)).toHaveLength(4096);
  const boundary = boundedConversationQuote("a".repeat(4095) + "€");
  expect(boundary).toBe("a".repeat(4095));
  expect(boundary).not.toContain("�");
});

it("launches a bounded quote from one host-bound released message and preserves native selection", async () => {
  const onOpen = await render();
  select(body(), body(), "😀".repeat(2000));
  await selectionChanged();
  const button = launcher()!;
  expect(button).toBeDefined();
  button.focus();
  expect(document.activeElement).toBe(button);
  const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
  expect(button.dispatchEvent(down)).toBe(false);
  await act(() => button.click());
  expect(onOpen).toHaveBeenCalledExactlyOnceWith({
    source: { messageId: "message-one", taskId: "released-task" },
    quote: "😀".repeat(1024),
  });
  expect(removeAllRanges).not.toHaveBeenCalled();
  expect(launcher()).toBeUndefined();
  await selectionChanged();
  expect(launcher()).toBeUndefined();
  select(body(), body(), "A different quote", 1);
  await selectionChanged();
  expect(launcher()).toBeDefined();
});

it("rejects cross-message, unmarked, outside, hidden, empty, and multi-range selections", async () => {
  await render();
  const rejected = [
    () => select(body(), body("message-two")),
    () => select(body("unreleased")),
    () => select(body("outside")),
    () => select(body(), body(), " \n "),
    () => {
      body().hidden = true;
      select(body());
    },
    () => {
      body().hidden = false;
      select(body());
      Object.assign(nativeSelection!, { rangeCount: 2 });
    },
  ];
  for (const prepare of rejected) {
    prepare();
    await selectionChanged();
    expect(launcher()).toBeUndefined();
  }
});

it("keeps selection through polling rerenders and resolves current host authority on activation", async () => {
  const onOpen = vi.fn();
  await render({ onOpen, resolveSource: (element) => sourceFor(element) });
  select(body());
  await selectionChanged();
  const initial = launcher();
  await render({
    onOpen,
    resolveSource: (element) => ({
      ...sourceFor(element)!,
      taskId: "latest-authorized-task",
    }),
  });
  expect(launcher()).toBe(initial);
  await act(() => launcher()!.click());
  expect(onOpen).toHaveBeenCalledExactlyOnceWith({
    source: { messageId: "message-one", taskId: "latest-authorized-task" },
    quote: "Released answer",
  });
  select(body(), body(), "New selection", 1);
  await selectionChanged();
  await render({ onOpen, resolveSource: () => null });
  await act(() => launcher()!.click());
  expect(onOpen).toHaveBeenCalledTimes(1);
  expect(removeAllRanges).not.toHaveBeenCalled();
});

it("revalidates the live range before opening a side chat", async () => {
  const onOpen = await render();
  select(body());
  await selectionChanged();
  select(body("message-two"));
  await act(() => launcher()!.click());
  expect(onOpen).not.toHaveBeenCalled();
  expect(launcher()).toBeUndefined();
});

it("returns keyboard focus before dismissing or opening from the launcher", async () => {
  const focusAtOpen: Element[] = [];
  await render({ onOpen: () => focusAtOpen.push(document.activeElement!) });
  const composer = container.querySelector<HTMLInputElement>("input")!;
  composer.focus();
  select(body());
  await selectionChanged();
  launcher()!.focus();
  await act(() => {
    launcher()!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
  });
  expect(launcher()).toBeUndefined();
  expect(document.activeElement).toBe(composer);
  select(body(), body(), "Another quote", 1);
  await selectionChanged();
  launcher()!.focus();
  await act(() => launcher()!.click());
  expect(focusAtOpen).toEqual([composer]);
  expect(document.activeElement).toBe(composer);
  expect(removeAllRanges).not.toHaveBeenCalled();
});

it("uses the selected readable message as a temporary focus fallback", async () => {
  await render();
  select(body());
  await selectionChanged();
  launcher()!.focus();
  await act(() => {
    window.dispatchEvent(new Event("scroll"));
  });
  expect(launcher()).toBeUndefined();
  expect(document.activeElement).toBe(body());
  expect(body().getAttribute("tabindex")).toBe("-1");
  container.querySelector<HTMLInputElement>("input")!.focus();
  expect(body().hasAttribute("tabindex")).toBe(false);
  expect(removeAllRanges).not.toHaveBeenCalled();
});

it("revalidates a captured range when keyboard focus collapses WebKit native selection", async () => {
  const onOpen = await render();
  select(body());
  await selectionChanged();
  launcher()!.focus();
  nativeSelection = null;
  await selectionChanged();
  expect(launcher()).toBeDefined();
  await act(() => launcher()!.click());
  expect(onOpen).toHaveBeenCalledExactlyOnceWith({
    source: { messageId: "message-one", taskId: "released-task" },
    quote: "Released answer",
  });
  select(body(), body(), "Another quote", 1);
  await selectionChanged();
  launcher()!.focus();
  Object.assign(nativeSelection!, { rangeCount: 2 });
  await selectionChanged();
  expect(launcher()).toBeUndefined();
  select(body(), body(), "Another quote", 2);
  await selectionChanged();
  launcher()!.focus();
  nativeSelection = null;
  body().firstChild!.textContent = "Changed released text";
  await selectionChanged();
  await act(() => launcher()!.click());
  expect(onOpen).toHaveBeenCalledTimes(1);
});

it("clears on Escape, scroll, resize, collapsed selection, and scope changes without removing selected text", async () => {
  const onOpen = await render();
  let offset = 0;
  for (const invalidate of [
    () =>
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })),
    () => window.dispatchEvent(new Event("scroll")),
    () => window.dispatchEvent(new Event("resize")),
  ]) {
    select(body(), body(), "Released answer", offset++);
    await selectionChanged();
    expect(launcher()).toBeDefined();
    await act(() => {
      invalidate();
    });
    expect(launcher()).toBeUndefined();
    await selectionChanged();
    expect(launcher()).toBeUndefined();
  }
  select(body(), body(), "Released answer", offset++);
  await selectionChanged();
  nativeSelection = null;
  await selectionChanged();
  expect(launcher()).toBeUndefined();
  select(body(), body(), "Released answer", offset++);
  await selectionChanged();
  await render({ onOpen, scopeKey: "thread-two" });
  expect(launcher()).toBeUndefined();
  await selectionChanged();
  expect(launcher()).toBeUndefined();
  select(body(), body(), "Released answer", offset++);
  await selectionChanged();
  expect(launcher()).toBeDefined();
  await render({ onOpen, scopeKey: "thread-two", enabled: false });
  expect(launcher()).toBeUndefined();
  expect(removeAllRanges).not.toHaveBeenCalled();
});
