import { useEffect, useId, useRef } from "react";
import type { ManagedSpaceConfigurationSnapshot } from "../types";

export function McpDeleteDialog({
  label,
  consumers,
  busy,
  error,
  returnFocus,
  onCancel,
  onDelete,
}: {
  label: string;
  consumers: ManagedSpaceConfigurationSnapshot[];
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
      const target = returnFocus?.isConnected
        ? returnFocus
        : document.getElementById("add-mcp-server");
      target?.focus();
    };
  }, [returnFocus]);

  return (
    <dialog
      ref={dialog}
      className="settings-dialog mcp-delete-dialog"
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
      <div className="mcp-delete-content" aria-busy={busy}>
        <p id={descriptionId}>
          This permanently removes the saved MCP server and all its versions.
          Saved credentials and OAuth data will be kept.
        </p>
        {consumers.length ? (
          <div>
            <p>
              Disable this server and apply changes in each workspace below
              before deleting it. Restore archived workspaces first.
            </p>
            <ul>
              {consumers.map((space) => (
                <li key={space.id}>
                  {space.name}
                  {space.archived ? " (archived)" : ""}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        {error ? <p role="alert">{error}</p> : null}
        {busy ? <p role="status">Deleting MCP server…</p> : null}
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
          disabled={busy || consumers.length > 0}
          onClick={onDelete}
        >
          Delete server
        </button>
      </footer>
    </dialog>
  );
}
