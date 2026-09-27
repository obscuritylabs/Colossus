import { useEffect, useId, useRef } from "react";
import type { DeletableCatalogKind } from "../catalog-deletion";

export function CatalogDeleteDialog({
  kind,
  label,
  blockers,
  busy,
  error,
  returnFocus,
  onCancel,
  onDelete,
}: {
  kind: DeletableCatalogKind;
  label: string;
  blockers: string[];
  busy: boolean;
  error: string;
  returnFocus: HTMLElement | null;
  onCancel: () => void;
  onDelete: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const headingId = useId();
  const descriptionId = useId();
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    cancel.current?.focus();
    return () => {
      element.close();
      // A successful save removes the row before the editor closes and re-enables Add.
      requestAnimationFrame(() => {
        if (document.querySelector("dialog[open]")) return;
        const target = returnFocus?.isConnected
          ? returnFocus
          : document.getElementById(`add-${kind}`);
        target?.focus();
      });
    };
  }, [kind, returnFocus]);
  return (
    <dialog
      ref={dialog}
      className="settings-dialog catalog-delete-dialog"
      aria-labelledby={headingId}
      aria-describedby={descriptionId}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onCancel();
      }}
    >
      <header>
        <h3 id={headingId}>Delete {label}?</h3>
      </header>
      <div className="catalog-delete-content" aria-busy={busy}>
        <p id={descriptionId}>
          This permanently removes the saved {kind} and all its versions from
          Colossus.
          {kind === "provider"
            ? " Saved credentials and your provider account will be kept."
            : " The provider connection will be kept."}
        </p>
        {blockers.length ? (
          <div>
            <p>
              Update or remove these references before deleting this {kind}.
              Apply workspace changes first, and restore archived workspaces if
              needed.
            </p>
            <ul>
              {blockers.map((blocker) => (
                <li key={blocker}>{blocker}</li>
              ))}
            </ul>
          </div>
        ) : null}
        {error ? <p role="alert">{error}</p> : null}
        {busy ? <p role="status">Deleting {kind}…</p> : null}
      </div>
      <footer>
        <button
          ref={cancel}
          className="button secondary"
          type="button"
          disabled={busy}
          onClick={onCancel}
        >
          Cancel
        </button>
        <button
          className="button danger"
          type="button"
          disabled={busy || blockers.length > 0}
          onClick={onDelete}
        >
          Delete {kind}
        </button>
      </footer>
    </dialog>
  );
}
