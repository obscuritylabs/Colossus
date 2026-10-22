import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import { Button } from "../components/ui/button.js";
import { boundedConversationQuote } from "./selection-quote.js";

const SELECTABLE_MESSAGE = "[data-aside-selectable='true']";

export interface ConversationSelectionQuote<TSource> {
  source: TSource;
  quote: string;
}

export interface ConversationSelectionBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
}

export interface ConversationSelection<
  TSource,
> extends ConversationSelectionQuote<TSource> {
  bounds: ConversationSelectionBounds;
}

export interface ConversationSelectionOptions<TSource> {
  containerRef: RefObject<HTMLElement | null>;
  scopeKey: string;
  enabled?: boolean;
  /** Only the host can bind a released message to its source identity. */
  resolveSource: (selectable: HTMLElement) => TSource | null;
}

interface RangeIdentity {
  startContainer: Node;
  startOffset: number;
  endContainer: Node;
  endOffset: number;
}

interface CapturedSelection<TSource> extends ConversationSelection<TSource> {
  scopeKey: string;
  selectable: HTMLElement;
  range: RangeIdentity;
  rangeText: string;
}

function sameRange(left: RangeIdentity, right: RangeIdentity): boolean {
  return (
    left.startContainer === right.startContainer &&
    left.startOffset === right.startOffset &&
    left.endContainer === right.endContainer &&
    left.endOffset === right.endOffset
  );
}

function rangeIdentity(selection: Selection | null): RangeIdentity | null {
  if (!selection || selection.rangeCount !== 1 || selection.isCollapsed)
    return null;
  const range = selection.getRangeAt(0);
  return {
    startContainer: range.startContainer,
    startOffset: range.startOffset,
    endContainer: range.endContainer,
    endOffset: range.endOffset,
  };
}

function collapsedSelection(selection: Selection | null): boolean {
  return (
    !selection ||
    selection.rangeCount === 0 ||
    (selection.rangeCount === 1 && selection.isCollapsed)
  );
}

function elementFor(node: Node | null): Element | null {
  return node?.nodeType === 1
    ? (node as Element)
    : (node?.parentElement ?? null);
}

function readSelection<TSource>(
  container: HTMLElement | null,
  scopeKey: string,
  resolveSource: (selectable: HTMLElement) => TSource | null,
): CapturedSelection<TSource> | null {
  if (!container?.isConnected) return null;
  const ownerWindow = container.ownerDocument.defaultView;
  const selection = ownerWindow?.getSelection() ?? null;
  const identity = rangeIdentity(selection);
  if (!ownerWindow || !selection || !identity) return null;
  const selectable = elementFor(selection.anchorNode)?.closest<HTMLElement>(
    SELECTABLE_MESSAGE,
  );
  const focusSelectable = elementFor(selection.focusNode)?.closest<HTMLElement>(
    SELECTABLE_MESSAGE,
  );
  if (!selectable || selectable !== focusSelectable) return null;
  return readMessageRange(
    container,
    selectable,
    selection.getRangeAt(0),
    selection.toString(),
    scopeKey,
    resolveSource,
  );
}

function readMessageRange<TSource>(
  container: HTMLElement,
  selectable: HTMLElement,
  range: Range,
  selectedText: string,
  scopeKey: string,
  resolveSource: (selectable: HTMLElement) => TSource | null,
): CapturedSelection<TSource> | null {
  const ownerWindow = container.ownerDocument.defaultView;
  if (
    !ownerWindow ||
    !container.isConnected ||
    !container.contains(selectable) ||
    !selectable.contains(range.startContainer) ||
    !selectable.contains(range.endContainer) ||
    selectable.closest("[hidden], [inert], [aria-hidden='true']")
  )
    return null;
  const source = resolveSource(selectable);
  if (source === null) return null;
  const quote = boundedConversationQuote(selectedText);
  if (quote === "") return null;
  const rect = range.getBoundingClientRect();
  const bounds = {
    left: rect.left,
    right: rect.right,
    top: rect.top,
    bottom: rect.bottom,
    width: rect.width,
    height: rect.height,
  };
  if (
    !Object.values(bounds).every(Number.isFinite) ||
    rect.width <= 0 ||
    rect.height <= 0 ||
    rect.bottom <= 0 ||
    rect.right <= 0 ||
    rect.top >= ownerWindow.innerHeight ||
    rect.left >= ownerWindow.innerWidth
  ) {
    return null;
  }
  return {
    source,
    quote,
    bounds,
    scopeKey,
    selectable,
    range: {
      startContainer: range.startContainer,
      startOffset: range.startOffset,
      endContainer: range.endContainer,
      endOffset: range.endOffset,
    },
    rangeText: boundedConversationQuote(range.toString()),
  };
}

function readCapturedRange<TSource>(
  container: HTMLElement | null,
  previous: CapturedSelection<TSource>,
  resolveSource: (selectable: HTMLElement) => TSource | null,
): CapturedSelection<TSource> | null {
  if (!container) return null;
  const range = container.ownerDocument.createRange();
  range.setStart(previous.range.startContainer, previous.range.startOffset);
  range.setEnd(previous.range.endContainer, previous.range.endOffset);
  if (boundedConversationQuote(range.toString()) !== previous.rangeText)
    return null;
  return readMessageRange(
    container,
    previous.selectable,
    range,
    previous.quote,
    previous.scopeKey,
    resolveSource,
  );
}

/** Selection presentation only; the host owns source identity and side-chat effects. */
export function useConversationSelection<TSource>(
  {
    containerRef,
    scopeKey,
    enabled = true,
    resolveSource,
  }: ConversationSelectionOptions<TSource>,
  actionRef?: RefObject<HTMLElement | null>,
): {
  selection: ConversationSelection<TSource> | null;
  clear: () => void;
  takeSelection: () => ConversationSelectionQuote<TSource> | null;
} {
  const [captured, setCaptured] = useState<CapturedSelection<TSource> | null>(
    null,
  );
  const capturedRef = useRef<CapturedSelection<TSource> | null>(null);
  const resolveSourceRef = useRef(resolveSource);
  useLayoutEffect(() => {
    resolveSourceRef.current = resolveSource;
  }, [resolveSource]);
  const dismissedRange = useRef<RangeIdentity | null>(null);
  const scheduled = useRef<{ window: Window; id: number } | null>(null);
  const cancelCapture = useCallback(() => {
    if (scheduled.current) {
      scheduled.current.window.cancelAnimationFrame(scheduled.current.id);
      scheduled.current = null;
    }
  }, []);
  const clear = useCallback(() => {
    cancelCapture();
    if (capturedRef.current) dismissedRange.current = capturedRef.current.range;
    else {
      try {
        dismissedRange.current = rangeIdentity(
          containerRef.current?.ownerDocument.defaultView?.getSelection() ??
            null,
        );
      } catch {
        dismissedRange.current = null;
      }
    }
    capturedRef.current = null;
    setCaptured(null);
  }, [containerRef, cancelCapture]);
  const capture = useCallback(() => {
    scheduled.current = null;
    try {
      const native =
        containerRef.current?.ownerDocument.defaultView?.getSelection() ?? null;
      const currentRange = rangeIdentity(native);
      // WebKit can collapse native text selection when a keyboard user focuses
      // its action. Retain the bound range only while that action owns focus.
      if (
        collapsedSelection(native) &&
        capturedRef.current &&
        actionRef?.current &&
        actionRef.current === actionRef.current.ownerDocument.activeElement
      )
        return;
      if (dismissedRange.current) {
        if (currentRange && sameRange(currentRange, dismissedRange.current))
          return;
        dismissedRange.current = null;
      }
      const next = enabled
        ? readSelection(
            containerRef.current,
            scopeKey,
            resolveSourceRef.current,
          )
        : null;
      const previous = capturedRef.current;
      if (
        next &&
        previous &&
        next.scopeKey === previous.scopeKey &&
        next.selectable === previous.selectable &&
        sameRange(next.range, previous.range) &&
        next.quote === previous.quote &&
        next.bounds.left === previous.bounds.left &&
        next.bounds.top === previous.bounds.top &&
        next.bounds.width === previous.bounds.width &&
        next.bounds.height === previous.bounds.height
      )
        return;
      capturedRef.current = next;
      setCaptured(next);
    } catch {
      capturedRef.current = null;
      setCaptured(null);
    }
  }, [containerRef, scopeKey, enabled, actionRef]);
  useEffect(() => {
    clear();
  }, [scopeKey, enabled, clear]);
  useEffect(() => {
    const ownerDocument = containerRef.current?.ownerDocument;
    const ownerWindow = ownerDocument?.defaultView;
    if (!enabled || !ownerDocument || !ownerWindow) return;
    const schedule = () => {
      if (scheduled.current) return;
      scheduled.current = {
        window: ownerWindow,
        id: ownerWindow.requestAnimationFrame(capture),
      };
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") clear();
    };
    ownerDocument.addEventListener("selectionchange", schedule);
    ownerDocument.addEventListener("mouseup", schedule);
    ownerDocument.addEventListener("keyup", schedule);
    ownerDocument.addEventListener("keydown", escape);
    ownerWindow.addEventListener("scroll", clear, {
      capture: true,
      passive: true,
    });
    ownerWindow.addEventListener("resize", clear, { passive: true });
    return () => {
      cancelCapture();
      ownerDocument.removeEventListener("selectionchange", schedule);
      ownerDocument.removeEventListener("mouseup", schedule);
      ownerDocument.removeEventListener("keyup", schedule);
      ownerDocument.removeEventListener("keydown", escape);
      ownerWindow.removeEventListener("scroll", clear, true);
      ownerWindow.removeEventListener("resize", clear);
    };
  }, [containerRef, enabled, capture, clear, cancelCapture]);
  const takeSelection = useCallback(() => {
    const previous = capturedRef.current;
    if (!enabled || !previous || previous.scopeKey !== scopeKey) {
      clear();
      return null;
    }
    try {
      const native =
        containerRef.current?.ownerDocument.defaultView?.getSelection() ?? null;
      const current =
        collapsedSelection(native) &&
        actionRef?.current &&
        actionRef.current === actionRef.current.ownerDocument.activeElement
          ? readCapturedRange(
              containerRef.current,
              previous,
              resolveSourceRef.current,
            )
          : readSelection(
              containerRef.current,
              scopeKey,
              resolveSourceRef.current,
            );
      return current &&
        current.selectable === previous.selectable &&
        sameRange(current.range, previous.range) &&
        current.quote === previous.quote
        ? { source: current.source, quote: current.quote }
        : null;
    } catch {
      return null;
    } finally {
      clear();
    }
  }, [containerRef, enabled, scopeKey, clear, actionRef]);
  return {
    selection: enabled && captured?.scopeKey === scopeKey ? captured : null,
    clear,
    takeSelection,
  };
}

export interface ConversationSelectionLauncherProps<
  TSource,
> extends ConversationSelectionOptions<TSource> {
  onOpen: (selection: ConversationSelectionQuote<TSource>) => void;
  label?: string;
  className?: string;
}

/** A native keyboard-accessible action anchored to the selected released text. */
export function ConversationSelectionLauncher<TSource>({
  onOpen,
  label = "Ask in side chat",
  className = "",
  ...options
}: ConversationSelectionLauncherProps<TSource>) {
  const button = useRef<HTMLButtonElement>(null);
  const { selection, clear, takeSelection } = useConversationSelection(
    options,
    button,
  );
  const ownedFocus = useRef(false);
  const focusOrigin = useRef<HTMLElement | null>(null);
  const selectedBody = useRef<HTMLElement | null>(null);
  const wasSelected = useRef(false);
  const restoreFocus = () => {
    if (!ownedFocus.current) return;
    ownedFocus.current = false;
    const usable = (element: HTMLElement | null) =>
      element?.isConnected &&
      !element.closest("[hidden], [inert], [aria-hidden='true']");
    const target = usable(focusOrigin.current)
      ? focusOrigin.current
      : usable(selectedBody.current)
        ? selectedBody.current
        : null;
    if (!target) return;
    if (target === selectedBody.current && !target.hasAttribute("tabindex")) {
      target.tabIndex = -1;
      target.addEventListener(
        "blur",
        () => {
          if (target.getAttribute("tabindex") === "-1")
            target.removeAttribute("tabindex");
        },
        { once: true },
      );
    }
    target.focus({ preventScroll: true });
  };
  const [position, setPosition] = useState<{
    selection: ConversationSelection<TSource>;
    left: number;
    top: number;
  } | null>(null);
  useLayoutEffect(() => {
    const element = button.current;
    const ownerWindow = element?.ownerDocument.defaultView;
    if (!selection) {
      restoreFocus();
      wasSelected.current = false;
      return;
    }
    if (!element || !ownerWindow) return;
    if (!wasSelected.current) {
      const active = element.ownerDocument.activeElement;
      focusOrigin.current =
        active && active !== element.ownerDocument.body && active !== element
          ? (active as HTMLElement)
          : null;
    }
    wasSelected.current = true;
    selectedBody.current =
      elementFor(
        ownerWindow.getSelection()?.anchorNode ?? null,
      )?.closest<HTMLElement>(SELECTABLE_MESSAGE) ?? null;
    const rect = element.getBoundingClientRect();
    const width = rect.width || 160;
    const height = rect.height || 36;
    const left = Math.max(
      8,
      Math.min(
        selection.bounds.left + selection.bounds.width / 2 - width / 2,
        ownerWindow.innerWidth - width - 8,
      ),
    );
    const above = selection.bounds.top - height - 8;
    const top = Math.max(
      8,
      Math.min(
        above >= 8 ? above : selection.bounds.bottom + 8,
        ownerWindow.innerHeight - height - 8,
      ),
    );
    setPosition({ selection, left, top });
  }, [selection]);
  if (!selection) return null;
  const placed = position?.selection === selection ? position : null;
  return (
    <Button
      ref={button}
      type="button"
      variant="outline"
      size="sm"
      className={`shared-conversation-selection-launcher ${className}`}
      style={{
        position: "fixed",
        zIndex: 60,
        left: placed?.left ?? 8,
        top: placed?.top ?? 8,
        maxWidth: "calc(100vw - 16px)",
        visibility: placed ? "visible" : "hidden",
      }}
      onMouseDown={(event) => event.preventDefault()}
      onFocus={() => {
        ownedFocus.current = true;
      }}
      onBlur={() => {
        ownedFocus.current = false;
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          restoreFocus();
          clear();
        }
      }}
      onClick={() => {
        const quote = takeSelection();
        restoreFocus();
        if (quote) onOpen(quote);
      }}
    >
      {label}
    </Button>
  );
}
