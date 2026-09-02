import {
  IconAlertTriangle,
  IconCircleCheck,
  IconInfoCircle,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useRef, useState } from "react";

export type ToastTone = "success" | "info" | "error";

export interface ToastMessage {
  id: number;
  message: string;
  tone: ToastTone;
}

const MAX_VISIBLE_TOASTS = 3;
const DEFAULT_TOAST_DURATION_MS = 6_000;

export function useToastQueue() {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const nextId = useRef(0);

  const pushToast = useCallback(
    (message: string, tone: ToastTone = "success") => {
      nextId.current += 1;
      const toast = { id: nextId.current, message, tone };
      setToasts((current) => [
        ...current.slice(-(MAX_VISIBLE_TOASTS - 1)),
        toast,
      ]);
      return toast.id;
    },
    [],
  );

  const dismissToast = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  return { dismissToast, pushToast, toasts };
}

function ToastCard({
  toast,
  onDismiss,
}: {
  toast: ToastMessage;
  onDismiss: (id: number) => void;
}) {
  useEffect(() => {
    if (toast.tone === "error") return;
    const timeout = window.setTimeout(
      () => onDismiss(toast.id),
      DEFAULT_TOAST_DURATION_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [onDismiss, toast.id, toast.tone]);

  const Icon =
    toast.tone === "success"
      ? IconCircleCheck
      : toast.tone === "error"
        ? IconAlertTriangle
        : IconInfoCircle;

  return (
    <div className={`app-toast is-${toast.tone}`}>
      <Icon size={19} stroke={1.8} aria-hidden="true" />
      <span>{toast.message}</span>
      <button
        type="button"
        aria-label="Dismiss notification"
        onClick={() => onDismiss(toast.id)}
      >
        <IconX size={17} stroke={1.8} aria-hidden="true" />
      </button>
    </div>
  );
}

export function ToastRegion({
  toasts,
  onDismiss,
}: {
  toasts: readonly ToastMessage[];
  onDismiss: (id: number) => void;
}) {
  return (
    <div
      className="toast-region"
      role="region"
      aria-label="Notifications"
      aria-live="polite"
      aria-relevant="additions text"
    >
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
}
