import { useCallback, useEffect, useLayoutEffect } from "react";
import type { RefObject } from "react";

/** Fit the draft before paint; CSS owns the minimum and maximum height. */
export function useComposerAutosize(
  ref: RefObject<HTMLTextAreaElement | null>,
  value: string,
) {
  const resize = useCallback(() => {
    const textarea = ref.current;
    if (!textarea) return;
    const scrollTop = textarea.scrollTop;
    const availableHeight = composerAvailableHeight(textarea);
    if (availableHeight !== null)
      textarea.style.setProperty(
        "--composer-available-height",
        `${availableHeight}px`,
      );
    const overflowY = textarea.style.overflowY;
    textarea.style.overflowY = "hidden";
    textarea.style.height = "auto";
    const borderHeight = textarea.offsetHeight - textarea.clientHeight;
    textarea.style.height = `${textarea.scrollHeight + borderHeight}px`;
    textarea.style.overflowY = overflowY;
    textarea.scrollTop = scrollTop;
  }, [ref]);

  useLayoutEffect(resize, [resize, value]);

  useEffect(() => {
    const textarea = ref.current;
    if (!textarea) return;
    let width = textarea.clientWidth;
    let availableHeight = composerAvailableHeight(textarea);
    const observer = new ResizeObserver(() => {
      const nextAvailableHeight = composerAvailableHeight(textarea);
      // Ignore our own height changes; surrounding controls can change the cap.
      if (
        textarea.clientWidth === width &&
        nextAvailableHeight === availableHeight
      )
        return;
      width = textarea.clientWidth;
      availableHeight = nextAvailableHeight;
      resize();
    });
    observer.observe(textarea);
    for (const selector of [".work-thread", ".work-composer-dock"]) {
      const container = textarea.closest(selector);
      if (container) observer.observe(container);
    }
    const appearanceObserver = new MutationObserver(resize);
    appearanceObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-text-size"],
    });
    window.addEventListener("resize", resize);
    document.fonts.addEventListener("loadingdone", resize);
    return () => {
      observer.disconnect();
      appearanceObserver.disconnect();
      window.removeEventListener("resize", resize);
      document.fonts.removeEventListener("loadingdone", resize);
    };
  }, [ref, resize]);
}

function composerAvailableHeight(textarea: HTMLTextAreaElement): number | null {
  const thread = textarea.closest(".work-thread");
  const dock = textarea.closest(".work-composer-dock");
  if (!thread || !dock) return null;
  const controlsHeight =
    dock.getBoundingClientRect().height -
    textarea.getBoundingClientRect().height;
  // Leave conversation context visible alongside approvals, queues and the footer.
  const conversationHeight = Math.min(120, thread.clientHeight * 0.2);
  return Math.max(
    48,
    Math.floor(thread.clientHeight - controlsHeight - conversationHeight),
  );
}
