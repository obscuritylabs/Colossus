import { useEffect, useRef, type ReactNode } from "react";
import { Button } from "@colossus/ui";
import { IconArrowLeft } from "@tabler/icons-react";
import "./settings-editor.css";

/** Hosts bind this presentation state to a complete, shareable URL. */
export interface SettingsEditorLocation {
  action: "new" | "edit";
  id?: string;
  /** A nonsecret, reviewed connection destination; the host persists it in the URL. */
  target?: { kind: "provider" | "mcp" | "search"; id: string };
}

export interface SettingsEditorNavigation {
  editorLocation?: SettingsEditorLocation | undefined;
  onOpenEditor?: ((location: SettingsEditorLocation) => void) | undefined;
  onCloseEditor?: (() => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
}

/** Page chrome shared by URL-addressed Web forms; hosts retain navigation guards. */
export function SettingsEditorPage({
  title,
  description,
  backLabel,
  onBack,
  busy = false,
  showHeading = true,
  children,
}: {
  title: string;
  description?: string;
  backLabel: string;
  onBack: () => void;
  busy?: boolean;
  showHeading?: boolean;
  children: ReactNode;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, [title]);
  return (
    <section className="settings-editor-page" aria-label={title}>
      <Button variant="tertiary" disabled={busy} onClick={onBack}>
        <IconArrowLeft size={16} aria-hidden="true" />
        Back to {backLabel}
      </Button>
      <header>
        <h2
          ref={heading}
          tabIndex={-1}
          className={showHeading ? undefined : "sr-only"}
        >
          {title}
        </h2>
        {description ? <p>{description}</p> : null}
      </header>
      {children}
    </section>
  );
}

export function SettingsEditorUnavailable({
  backLabel,
  onBack,
  reason,
}: {
  backLabel: string;
  onBack: () => void;
  reason?: string | undefined;
}) {
  return (
    <SettingsEditorPage
      title="Setting unavailable"
      backLabel={backLabel}
      onBack={onBack}
    >
      <p role="status">
        {reason ??
          "This setting was removed or is unavailable in this scope. Return to the list to choose a current item."}
      </p>
    </SettingsEditorPage>
  );
}
