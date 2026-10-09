import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

export function SetupReviewDialog({
  children,
  returnFocus,
  busy,
  onClose,
}: {
  children: ReactNode;
  returnFocus: HTMLElement | null;
  busy: boolean;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => {
      dialog.close();
      requestAnimationFrame(() => returnFocus?.focus());
    };
  }, [returnFocus]);
  return createPortal(
    <dialog
      ref={ref}
      className="settings-dialog setup-import-dialog"
      aria-labelledby="setup-review-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      {children}
    </dialog>,
    document.body,
  );
}
