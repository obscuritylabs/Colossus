import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import type { AppshotPreview as Preview } from "../desktop-extras";
import "./desktop-extras.css";

export function AppshotPreview({
  capture,
  busy,
  canAttach,
  error,
  onAttach,
  onDiscard,
}: {
  capture: Preview;
  busy: boolean;
  canAttach: boolean;
  error: string;
  onAttach: (includeText: boolean) => void;
  onDiscard: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [includeText, setIncludeText] = useState(capture.text !== null);
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) {
      if (typeof dialog.showModal === "function") dialog.showModal();
      else dialog.setAttribute("open", "");
    }
    return () => {
      dialog?.close?.();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className="appshot-preview"
      aria-labelledby="appshot-heading"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onDiscard();
      }}
    >
      <header>
        <div>
          <h3 id="appshot-heading">App Shot</h3>
          <p>
            {capture.windowTitle || "Captured app window"} · {capture.width} ×{" "}
            {capture.height}
          </p>
        </div>
        <Button type="button" disabled={busy} onClick={onDiscard}>
          Discard
        </Button>
      </header>
      <div className="appshot-preview-body">
        <img
          src={capture.imageDataUrl}
          alt={`Captured window: ${capture.windowTitle || "app"}`}
        />
        {capture.text !== null ? (
          <details>
            <summary>View captured text</summary>
            <pre tabIndex={0}>{capture.text}</pre>
          </details>
        ) : (
          <p className="muted">
            This app did not provide visible text. The screenshot is available.
          </p>
        )}
      </div>
      <footer>
        {capture.text !== null && (
          <label>
            <input
              type="checkbox"
              checked={includeText}
              disabled={busy}
              onChange={(e) => setIncludeText(e.target.checked)}
            />{" "}
            Include captured text
          </label>
        )}
        <p>
          The capture is added to this chat’s draft. Review your message before
          sending.
        </p>
        {error && <p role="alert">{error}</p>}
        <Button
          type="button"
          disabled={!canAttach || busy}
          onClick={() => onAttach(includeText)}
        >
          {busy ? "Attaching…" : "Attach to current draft"}
        </Button>
      </footer>
    </dialog>
  );
}
