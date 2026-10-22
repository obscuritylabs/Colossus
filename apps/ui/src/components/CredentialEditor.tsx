import {
  useEffect,
  useId,
  useRef,
  type ReactNode,
  type KeyboardEvent,
} from "react";
import { IconKey, IconLock, IconX } from "@tabler/icons-react";
import { Button } from "./Controls.js";
import "../../styles/provider-settings-editor.css";
import "../../styles/provider-setup.css";

/** Hosts own secure entry, credential scope, and persistence. */
export function CredentialEditor({
  title = "Add credential",
  description,
  busy,
  onCancel,
  onSubmit,
  submitDisabled = false,
  submitLabel = "Continue to secure entry",
  children,
}: {
  title?: string;
  description: string;
  busy: boolean;
  onCancel: () => void;
  onSubmit?: (() => void) | undefined;
  submitDisabled?: boolean;
  submitLabel?: string;
  children: ReactNode;
}) {
  const id = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const trigger = useRef(
    typeof document === "undefined" ? null : document.activeElement,
  );
  useEffect(() => {
    const first = heading.current
      ?.closest(".shared-credential-editor")
      ?.querySelector<HTMLElement>(
        ".shared-credential-editor-body input:not(:disabled), .shared-credential-editor-body button[role=combobox]:not(:disabled)",
      );
    (first ?? heading.current)?.focus();
    return () => {
      if (trigger.current instanceof HTMLElement && trigger.current.isConnected)
        trigger.current.focus();
    };
  }, []);
  const content = (
    <>
      <header className="provider-editor-heading">
        <span className="resource-icon">
          <IconKey size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 id={id} ref={heading} tabIndex={-1}>
            {title}
          </h4>
          <p>{description}</p>
        </div>
        <Button
          className="ui-icon-button"
          aria-label="Close credential editor"
          disabled={busy}
          onClick={onCancel}
        >
          <IconX size={17} aria-hidden="true" />
        </Button>
      </header>
      <div className="shared-credential-editor-body provider-editor-section provider-credential-control">
        {children}
      </div>
      <footer className="mcp-editor-actions">
        <Button disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        {onSubmit ? (
          <Button
            type="submit"
            variant="primary"
            disabled={busy || submitDisabled}
          >
            <IconLock size={16} aria-hidden="true" /> {submitLabel}
          </Button>
        ) : null}
      </footer>
    </>
  );
  const props = {
    className: "shared-credential-editor shared-provider-settings-editor",
    "aria-labelledby": id,
    onKeyDown: (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented && !busy) {
        event.preventDefault();
        onCancel();
      }
    },
  };
  return onSubmit ? (
    <form
      {...props}
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy && !submitDisabled) onSubmit();
      }}
    >
      {content}
    </form>
  ) : (
    <section {...props}>{content}</section>
  );
}
