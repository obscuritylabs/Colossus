import { CredentialForm } from "@colossus/ui/components/CredentialForm";
import { request } from "./api";

/** Personal writes are bound to the endpoint reviewed by this user. */
export function ChatConnectionCredential({
  path,
  binding,
  endpoint,
  disabled,
  onSaved,
  onError,
  onBusyChange,
}: {
  path: string;
  binding: string | null;
  endpoint: string;
  disabled: boolean;
  onSaved: () => void;
  onError: (error: unknown) => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  return (
    <CredentialForm
      key={JSON.stringify([path, binding, endpoint])}
      scope={JSON.stringify([path, binding, endpoint])}
      label="Your API key or token"
      action="Save personal credential"
      endpoint={endpoint}
      disabled={disabled || !binding}
      description="Encrypted in server storage and used only by your personal chats. The saved value cannot be read back."
      onSave={async (secret) => {
        onBusyChange?.(true);
        try {
          return await request(path, { secret, credential_binding: binding });
        } finally {
          onBusyChange?.(false);
        }
      }}
      onSaved={onSaved}
      onError={onError}
    />
  );
}
