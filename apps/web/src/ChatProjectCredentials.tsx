import type { CredentialTarget } from "./credential-types";
import { CredentialStoreSettings } from "./CredentialStoreSettings";
export function ChatProjectCredentials({
  path,
  identity,
  disabled,
  onSaved,
  initialConnection,
}: {
  path: string;
  identity: string;
  disabled: boolean;
  onSaved: () => void;
  initialConnection?: CredentialTarget | undefined;
}) {
  return (
    <CredentialStoreSettings
      path={path.replace(/\/settings$/, "/credential-store/project")}
      identity={identity}
      canEdit={!disabled}
      projectScope
      onSaved={onSaved}
      initialConnection={initialConnection}
    />
  );
}
