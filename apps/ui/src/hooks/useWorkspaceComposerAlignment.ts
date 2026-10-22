import { useEffect, type RefObject } from "react";

/** Align intrinsic composer docks while hosts keep editor state and submission. */
export function useWorkspaceComposerAlignment(
  root: RefObject<HTMLElement | null>,
  enabled: boolean,
) {
  useEffect(() => {
    const element = root.current;
    if (!element || !enabled) return;
    let frame = 0;
    const observed = new Set<Element>();
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    const resize = new ResizeObserver(schedule);
    const measure = () => {
      const docks = [
        ...element.querySelectorAll<HTMLElement>(
          "[data-workspace-composer-dock]",
        ),
      ];
      const targets = new Set<Element>([element]);
      for (const dock of docks) {
        targets.add(dock);
        for (const child of dock.querySelectorAll(
          "form, textarea, [contenteditable], [data-workspace-composer-measure], .shared-composer-header, .shared-composer-footer",
        ))
          targets.add(child);
      }
      for (const target of observed)
        if (!targets.has(target)) {
          resize.unobserve(target);
          observed.delete(target);
        }
      for (const target of targets)
        if (!observed.has(target)) {
          resize.observe(target);
          observed.add(target);
        }
      // Remove the previous minimum before measuring so shorter drafts can shrink.
      // Both writes happen before paint; observer work is deferred to the next frame.
      element.style.removeProperty("--workspace-composer-dock-height");
      const visible = docks.filter(
        (dock) =>
          dock.getBoundingClientRect().width > 0 &&
          dock.getBoundingClientRect().height > 0,
      );
      if (visible.length < 2) return;
      const height = Math.ceil(
        Math.max(...visible.map((dock) => dock.getBoundingClientRect().height)),
      );
      element.style.setProperty(
        "--workspace-composer-dock-height",
        `${height}px`,
      );
    };
    const mutation = new MutationObserver((records) => {
      const affectsDock = records.some((record) => {
        const target =
          record.target instanceof Element
            ? record.target
            : record.target.parentElement;
        if (target?.closest("[data-workspace-composer-dock]")) return true;
        if (record.type === "attributes")
          return Boolean(
            target?.querySelector("[data-workspace-composer-dock]"),
          );
        return [...record.addedNodes, ...record.removedNodes].some(
          (node) =>
            node instanceof Element &&
            (node.matches("[data-workspace-composer-dock]") ||
              node.querySelector("[data-workspace-composer-dock]")),
        );
      });
      if (affectsDock) schedule();
    });
    mutation.observe(element, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["hidden", "class"],
    });
    schedule();
    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
      mutation.disconnect();
      element.style.removeProperty("--workspace-composer-dock-height");
    };
  }, [root, enabled]);
}
