import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";
import { ModelPicker } from "@colossus/ui/model-picker";

/** Keep composer model/reasoning controls inside their containing inspector sheet. */
export function ComposerModelPicker(
  props: Omit<ComponentProps<typeof ModelPicker>, "portalContainer">,
) {
  const wrapper = useRef<HTMLSpanElement>(null);
  const [portalContainer, setPortalContainer] = useState<HTMLElement | null>(
    null,
  );
  function preparePortal() {
    setPortalContainer(
      wrapper.current?.closest<HTMLElement>('[data-slot="sheet-content"]') ??
        null,
    );
  }
  useLayoutEffect(preparePortal, []);
  useEffect(() => {
    const ownerDocument = wrapper.current?.ownerDocument;
    const ownerWindow = ownerDocument?.defaultView;
    if (!ownerDocument || !ownerWindow) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing) return;
      const sheet = wrapper.current?.closest<HTMLElement>(
        '[data-slot="sheet-content"]',
      );
      const trigger = wrapper.current?.querySelector<HTMLButtonElement>(
        'button[aria-haspopup="dialog"][aria-expanded="true"]',
      );
      const contentId = trigger?.getAttribute("aria-controls");
      const content = contentId
        ? ownerDocument.getElementById(contentId)
        : null;
      if (
        !sheet ||
        !content ||
        !sheet.contains(content) ||
        !(event.target instanceof Node) ||
        !content.contains(event.target)
      )
        return;
      // The sheet and popover have separate document-level Radix layer stacks.
      // Cancel ancestor dismissal; ModelPicker's explicit Escape callback still
      // closes its own popover and preserves Radix's trigger focus restoration.
      event.preventDefault();
    };
    ownerWindow.addEventListener("keydown", escape, true);
    return () => ownerWindow.removeEventListener("keydown", escape, true);
  }, []);
  return (
    <span
      ref={wrapper}
      style={{ display: "contents" }}
      onPointerDownCapture={preparePortal}
      onKeyDownCapture={preparePortal}
    >
      <ModelPicker {...props} portalContainer={portalContainer} />
    </span>
  );
}
