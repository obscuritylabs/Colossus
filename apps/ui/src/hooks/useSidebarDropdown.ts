import { useCallback, useEffect, useRef, useState } from "react";

export function useSidebarDropdown() {
  const [open, setOpen] = useState(false);
  const [portalContainer, setPortalContainer] = useState<HTMLElement | null>(
    null,
  );
  const triggerRef = useRef<HTMLButtonElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const onOpenChange = useCallback((nextOpen: boolean) => {
    if (nextOpen) {
      // Keep mobile menu focus inside the containing sheet's focus trap.
      setPortalContainer(
        triggerRef.current?.closest<HTMLElement>(
          '[data-slot="sheet-content"]',
        ) ?? null,
      );
    }
    setOpen(nextOpen);
  }, []);
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.key !== "Escape" ||
        event.isComposing ||
        !(event.target instanceof Node) ||
        !contentRef.current?.contains(event.target)
      ) {
        return;
      }
      // Menu and sheet primitives have separate document-level layer stacks.
      // Close this portalled menu before Escape reaches its containing sheet.
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [open]);
  return {
    open,
    onOpenChange,
    triggerRef,
    contentRef,
    portalContainer,
    // The containing sheet owns modal focus and pointer containment. A second
    // modal layer can restore a stale body pointer lock when navigation unmounts both.
    modal: portalContainer === null,
  };
}
