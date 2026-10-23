import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, TextInput } from "./Controls.js";
import "../../styles/credentials.css";

/** Write-only presentation. Hosts own credential scope, storage and authorization. */
export function CredentialForm({
  scope,
  label,
  action,
  description,
  endpoint,
  disabled = false,
  submitDisabled = false,
  onSave,
  onSaved,
  onError,
}: {
  scope: string;
  label: string;
  action: string;
  description: string;
  endpoint: string;
  disabled?: boolean;
  submitDisabled?: boolean;
  onSave: (secret: string) => Promise<unknown>;
  onSaved: () => void;
  onError: (error: unknown) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const current = useRef(scope);
  const alive = useRef(true);
  const [busy, setBusy] = useState(false);
  useLayoutEffect(() => {
    current.current = scope;
    if (input.current) input.current.value = "";
  }, [scope, disabled]);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (input.current) input.current.value = "";
    };
  }, []);
  return (
    <form
      className="shared-credential-form"
      aria-label={label}
      onSubmit={(event) => {
        event.preventDefault();
        if (!input.current?.value || disabled || submitDisabled || busy) return;
        const secret = input.current.value;
        input.current.value = "";
        const submitted = scope;
        setBusy(true);
        void onSave(secret)
          .then(() => {
            if (alive.current && current.current === submitted) onSaved();
          })
          .catch((error: unknown) => {
            if (alive.current && current.current === submitted) onError(error);
          })
          .finally(() => {
            if (alive.current) setBusy(false);
          });
      }}
    >
      <label>
        <span>{label}</span>
        <TextInput
          ref={input}
          aria-label={label}
          type="password"
          autoComplete="new-password"
          maxLength={65536}
          required
          disabled={disabled || busy}
        />
      </label>
      <small>Credential destination: {endpoint || "Not reported"}</small>
      <small>{description}</small>
      <Button
        type="submit"
        variant="primary"
        disabled={disabled || submitDisabled || busy}
      >
        {busy ? "Saving…" : action}
      </Button>
    </form>
  );
}
