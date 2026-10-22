import { useEffect, useRef, useState } from "react";
import { IconCheck, IconCopy } from "@tabler/icons-react";

export interface MessageCopyButtonProps {
  text: string;
  label?: string;
  onCopy?: ((text: string) => void | Promise<void>) | undefined;
}

/** Clipboard writes happen only after an explicit human activation. */
async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    // Some embedded webviews do not expose the asynchronous Clipboard API.
  }
  const focused = document.activeElement;
  const selection = document.getSelection();
  const ranges = selection
    ? Array.from({ length: selection.rangeCount }, (_, index) =>
        selection.getRangeAt(index).cloneRange(),
      )
    : [];
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.readOnly = true;
  textarea.className = "shared-copy-buffer";
  document.body.append(textarea);
  try {
    textarea.select();
    if (!document.execCommand("copy")) throw new Error("Clipboard unavailable");
  } finally {
    textarea.remove();
    if (focused instanceof HTMLElement) focused.focus({ preventScroll: true });
    if (selection) {
      selection.removeAllRanges();
      for (const range of ranges) selection.addRange(range);
    }
  }
}

export function MessageCopyButton({
  text,
  label = "message",
  onCopy,
}: MessageCopyButtonProps) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const epoch = useRef(0);
  useEffect(() => {
    epoch.current++;
    setState("idle");
    return () => {
      epoch.current++;
      if (timer.current !== null) clearTimeout(timer.current);
    };
  }, [text]);
  async function copy() {
    const current = ++epoch.current;
    try {
      await (onCopy ?? copyText)(text);
      if (current === epoch.current) setState("copied");
    } catch {
      if (current === epoch.current) setState("failed");
    }
    if (current !== epoch.current) return;
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = setTimeout(() => setState("idle"), 2000);
  }
  const accessibleLabel =
    state === "copied"
      ? `Copied ${label}`
      : state === "failed"
        ? `Could not copy ${label}; retry`
        : `Copy ${label}`;
  return (
    <button
      className={`shared-message-copy is-${state}`}
      type="button"
      aria-label={accessibleLabel}
      title={accessibleLabel}
      disabled={text.trim() === ""}
      onClick={() => void copy()}
    >
      {state === "copied" ? (
        <IconCheck size={14} stroke={1.8} aria-hidden="true" />
      ) : (
        <IconCopy size={14} stroke={1.8} aria-hidden="true" />
      )}
      <span>
        {state === "copied" ? "Copied" : state === "failed" ? "Retry" : "Copy"}
      </span>
      <span className="shared-sr-only" aria-live="polite">
        {state === "copied"
          ? `${label} copied to clipboard.`
          : state === "failed"
            ? "Clipboard unavailable. Try copying again."
            : ""}
      </span>
    </button>
  );
}
