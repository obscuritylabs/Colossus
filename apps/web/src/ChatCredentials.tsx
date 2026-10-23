import type { CredentialTarget } from "./credential-types";
import type { SettingsEditorLocation } from "./settings-editor";
import { Button } from "@colossus/ui";
import { projectPath } from "./api";
import { CredentialStoreSettings } from "./CredentialStoreSettings";
export function ChatCredentials({
  project,
  identity,
  canConnect,
  onConnections,
  onSaved,
  initialConnection,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
}: {
  project: string;
  identity: string;
  canConnect: boolean;
  onConnections: () => void;
  onSaved: () => void;
  initialConnection?: CredentialTarget | undefined;
  editorLocation?: SettingsEditorLocation | undefined;
  onOpenEditor?: ((location: SettingsEditorLocation) => void) | undefined;
  onCloseEditor?: (() => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
}) {
  return (
    <>
      <CredentialStoreSettings
        path={`${projectPath(project)}/chat/credential-store/personal`}
        identity={identity}
        canEdit={canConnect}
        onSaved={onSaved}
        initialConnection={initialConnection}
        editorLocation={editorLocation}
        onOpenEditor={onOpenEditor}
        onCloseEditor={onCloseEditor}
        onDirtyChange={onDirtyChange}
      />
      {!editorLocation ? (
        <Button onClick={onConnections}>Manage connected accounts</Button>
      ) : null}
    </>
  );
}
