import type { CredentialTarget } from "./credential-types";
import type { SettingsEditorLocation } from "./settings-editor";
import { CredentialStoreSettings } from "./CredentialStoreSettings";
export function ChatProjectCredentials({
  path,
  identity,
  disabled,
  onSaved,
  initialConnection,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
}: {
  path: string;
  identity: string;
  disabled: boolean;
  onSaved: () => void;
  initialConnection?: CredentialTarget | undefined;
  editorLocation?: SettingsEditorLocation | undefined;
  onOpenEditor?: ((location: SettingsEditorLocation) => void) | undefined;
  onCloseEditor?: (() => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
}) {
  return (
    <CredentialStoreSettings
      path={path.replace(/\/settings$/, "/credential-store/project")}
      identity={identity}
      canEdit={!disabled}
      projectScope
      onSaved={onSaved}
      initialConnection={initialConnection}
      editorLocation={editorLocation}
      onOpenEditor={onOpenEditor}
      onCloseEditor={onCloseEditor}
      onDirtyChange={onDirtyChange}
    />
  );
}
