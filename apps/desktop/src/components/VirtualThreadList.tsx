import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";

const ESTIMATED_ROW_HEIGHT = 44;
const OVERSCAN = 320;

export function historyWindow(
  offsets: readonly number[],
  top: number,
  height: number,
) {
  const locate = (position: number) => {
    let low = 0,
      high = offsets.length - 1;
    while (low < high) {
      const middle = Math.ceil((low + high) / 2);
      if (offsets[middle]! <= position) low = middle;
      else high = middle - 1;
    }
    return low;
  };
  return {
    start: locate(Math.max(0, top - OVERSCAN)),
    end: Math.min(
      offsets.length - 1,
      locate(Math.max(0, top + height + OVERSCAN)) + 1,
    ),
  };
}

/** Measured rows, including rename forms and Large text; no content height is fixed. */
export function VirtualThreadList<T extends { runId: string }>({
  items,
  scrollRef,
  children,
}: {
  items: readonly T[];
  scrollRef: RefObject<HTMLDivElement | null>;
  children: (item: T) => ReactNode;
}) {
  const element = useRef<HTMLDivElement>(null);
  const heights = useRef(new Map<string, number>());
  const [revision, setRevision] = useState(0);
  const [viewport, setViewport] = useState({ top: 0, height: 640 });
  const [focused, setFocused] = useState<string | null>(null);
  const anchor = useRef<{ id: string; inset: number } | null>(null);
  const viewportAnchor = useRef<{ id: string; inset: number } | null>(null);
  const previousItems = useRef(items);
  const offsets = useMemo(() => {
    const result = [0];
    for (const item of items)
      result.push(
        result.at(-1)! +
          (heights.current.get(item.runId) ?? ESTIMATED_ROW_HEIGHT) +
          1,
      );
    return result;
  }, [items, revision]);
  const model = useRef({ items, offsets });
  model.current = { items, offsets };

  const updateViewport = useCallback(() => {
    if (!element.current || !scrollRef.current) return;
    const bounds = element.current.getBoundingClientRect();
    const scroll = scrollRef.current.getBoundingClientRect();
    const next = {
      top: scroll.top - bounds.top,
      height: scrollRef.current.clientHeight,
    };
    const { items: rows, offsets: positions } = model.current;
    const index = positions.findIndex(
      (offset, i) =>
        i < rows.length && positions[i + 1]! > next.top && offset <= next.top,
    );
    viewportAnchor.current =
      index < 0
        ? null
        : { id: rows[index]!.runId, inset: next.top - positions[index]! };
    setViewport((current) =>
      current.top === next.top && current.height === next.height
        ? current
        : next,
    );
  }, [scrollRef]);

  const rememberAnchor = useCallback(() => {
    if (!element.current || !scrollRef.current || anchor.current) return;
    const top =
      scrollRef.current.getBoundingClientRect().top -
      element.current.getBoundingClientRect().top;
    const { items: rows, offsets: positions } = model.current;
    const index = positions.findIndex(
      (offset, i) =>
        i < rows.length && positions[i + 1]! > top && offset <= top,
    );
    if (index >= 0)
      anchor.current = {
        id: rows[index]!.runId,
        inset: top - positions[index]!,
      };
  }, [scrollRef]);

  const measure = useCallback(
    (id: string, height: number) => {
      if (
        height <= 0 ||
        Math.abs((heights.current.get(id) ?? ESTIMATED_ROW_HEIGHT) - height) <
          0.5
      )
        return;
      rememberAnchor();
      heights.current.set(id, height);
      setRevision((current) => current + 1);
    },
    [rememberAnchor],
  );

  useLayoutEffect(() => {
    const scroller = scrollRef.current;
    const list = element.current;
    if (!scroller || !list) return;
    updateViewport();
    let frame = 0;
    const schedule = () => {
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          updateViewport();
        });
    };
    const resize = new ResizeObserver(() => {
      rememberAnchor();
      heights.current.clear();
      for (const row of list.querySelectorAll<HTMLElement>(
        "[data-history-row]",
      )) {
        heights.current.set(
          row.dataset.historyRow!,
          row.getBoundingClientRect().height,
        );
      }
      setRevision((current) => current + 1);
      schedule();
    });
    resize.observe(scroller);
    const preferences = new MutationObserver(() => {
      rememberAnchor();
      heights.current.clear();
      for (const row of list.querySelectorAll<HTMLElement>(
        "[data-history-row]",
      )) {
        heights.current.set(
          row.dataset.historyRow!,
          row.getBoundingClientRect().height,
        );
      }
      setRevision((current) => current + 1);
    });
    preferences.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-text-size"],
    });
    scroller.addEventListener("scroll", schedule, { passive: true });
    return () => {
      resize.disconnect();
      preferences.disconnect();
      scroller.removeEventListener("scroll", schedule);
      cancelAnimationFrame(frame);
    };
  }, [rememberAnchor, scrollRef, updateViewport]);

  useLayoutEffect(() => {
    const saved =
      anchor.current ??
      (previousItems.current !== items ? viewportAnchor.current : null);
    if (saved && element.current && scrollRef.current) {
      const index = items.findIndex((item) => item.runId === saved.id);
      if (index >= 0) {
        const top =
          scrollRef.current.getBoundingClientRect().top -
          element.current.getBoundingClientRect().top;
        scrollRef.current.scrollTop += offsets[index]! + saved.inset - top;
      }
      anchor.current = null;
    }
    previousItems.current = items;
    updateViewport();
  }, [items, offsets, scrollRef, updateViewport]);

  const window = historyWindow(offsets, viewport.top, viewport.height);
  const indices = new Set(
    Array.from(
      { length: Math.max(0, window.end - window.start) },
      (_, i) => window.start + i,
    ),
  );
  const focusedIndex = items.findIndex((item) => item.runId === focused);
  if (focusedIndex >= 0) indices.add(focusedIndex);

  return (
    <div
      ref={element}
      className="virtual-thread-list"
      role="list"
      style={{ height: offsets.at(-1) }}
      onFocusCapture={(event) => {
        const row = (event.target as HTMLElement).closest<HTMLElement>(
          "[data-history-row]",
        );
        if (row) setFocused(row.dataset.historyRow ?? null);
      }}
      onKeyDown={(event) => {
        if (
          event.key !== "Tab" ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey
        )
          return;
        const row = (event.target as HTMLElement).closest<HTMLElement>(
          "[data-history-row]",
        );
        if (!row) return;
        const index = items.findIndex(
          (item) => item.runId === row.dataset.historyRow,
        );
        const controls = [
          ...row.querySelectorAll<HTMLElement>(
            "button:not(:disabled), input:not(:disabled)",
          ),
        ];
        const edge = event.shiftKey ? controls[0] : controls.at(-1);
        const next = index + (event.shiftKey ? -1 : 1);
        if (event.target !== edge || next < 0 || next >= items.length) return;
        event.preventDefault();
        const id = items[next]!.runId;
        setFocused(id);
        requestAnimationFrame(() => {
          const nextRow = [
            ...(element.current?.querySelectorAll<HTMLElement>(
              "[data-history-row]",
            ) ?? []),
          ].find((candidate) => candidate.dataset.historyRow === id);
          const targets = [
            ...(nextRow?.querySelectorAll<HTMLElement>(
              "button:not(:disabled), input:not(:disabled)",
            ) ?? []),
          ];
          const target = event.shiftKey ? targets.at(-1) : targets[0];
          target?.focus({ preventScroll: true });
          nextRow?.scrollIntoView({ block: "nearest" });
        });
      }}
    >
      {[...indices]
        .sort((a, b) => a - b)
        .map((index) => (
          <MeasuredRow
            key={items[index]!.runId}
            id={items[index]!.runId}
            top={offsets[index]!}
            index={index}
            count={items.length}
            measure={measure}
          >
            {children(items[index]!)}
          </MeasuredRow>
        ))}
    </div>
  );
}

function MeasuredRow({
  id,
  top,
  index,
  count,
  measure,
  children,
}: {
  id: string;
  top: number;
  index: number;
  count: number;
  measure: (id: string, height: number) => void;
  children: ReactNode;
}) {
  const element = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (!element.current) return;
    const observer = new ResizeObserver(() => {
      if (element.current)
        measure(id, element.current.getBoundingClientRect().height);
    });
    observer.observe(element.current);
    return () => observer.disconnect();
  }, [id, measure]);
  return (
    <div
      ref={element}
      data-history-row={id}
      role="listitem"
      aria-posinset={index + 1}
      aria-setsize={count}
      className="virtual-thread-row"
      style={{ top }}
    >
      {children}
    </div>
  );
}
