import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

export function WorkflowDialog({
  title,
  busy,
  onClose,
  children,
  returnFocus,
  error,
  className = "",
  headerActions,
}: {
  title: string;
  busy: boolean;
  onClose: () => void;
  children: ReactNode;
  returnFocus?: HTMLElement | null;
  error?: string | undefined;
  className?: string;
  headerActions?: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const previous =
      returnFocus ??
      (document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null);
    const dialog = ref.current!;
    dialog.showModal();
    return () => {
      dialog.close();
      requestAnimationFrame(() => {
        if (previous?.isConnected) previous.focus();
      });
    };
  }, []);
  useEffect(() => {
    ref.current?.scrollTo({ top: 0 });
    document.getElementById(titleId)?.focus({ preventScroll: true });
  }, [title, titleId]);
  useEffect(() => {
    if (!error) return;
    const alert = ref.current?.querySelector<HTMLElement>('[role="alert"]');
    if (alert) {
      alert.tabIndex = -1;
      ref.current?.scrollTo({ top: 0 });
      alert.focus({ preventScroll: true });
    }
  }, [error]);
  return createPortal(
    <dialog
      ref={ref}
      className={`settings-dialog workflow-dialog ${className}`}
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      <header>
        <h3 id={titleId} tabIndex={-1}>
          {title}
        </h3>
        {headerActions}
      </header>
      <div className="workflow-dialog-content" aria-busy={busy}>
        {children}
      </div>
    </dialog>,
    document.body,
  );
}
