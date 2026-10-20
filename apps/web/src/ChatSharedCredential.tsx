import { CredentialForm } from "@colossus/ui/components/CredentialForm";
import { request } from "./api";
export function ChatSharedCredential({
  path,
  revision,
  endpoint,
  present,
  onSaved,
  onError,
  disabled,
  expanded = false,
}: {
  path: string;
  revision: number;
  endpoint: string;
  present: boolean;
  disabled: boolean;
  expanded?: boolean;
  onSaved: () => void;
  onError: (error: unknown) => void;
}) {
  const form = (
    <CredentialForm
      scope={JSON.stringify([path, revision, endpoint])}
      label="Project credential"
      action="Save credential"
      endpoint={endpoint}
      disabled={disabled}
      description="Shared by this project's chat users. The stored value cannot be read back."
      onSave={(secret) => request(path, { revision, secret })}
      onSaved={onSaved}
      onError={onError}
    />
  );
  return expanded ? (
    form
  ) : (
    <details className="chat-shared-credential">
      <summary>
        {present ? "Replace project credential" : "Set project credential"}
      </summary>
      {form}
    </details>
  );
}
