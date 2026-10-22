import type { CredentialTarget } from "./credential-types";
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
}: {
  project: string;
  identity: string;
  canConnect: boolean;
  onConnections: () => void;
  onSaved: () => void;
  initialConnection?: CredentialTarget | undefined;
}) {
  return (
    <>
      <CredentialStoreSettings
        path={`${projectPath(project)}/chat/credential-store/personal`}
        identity={identity}
        canEdit={canConnect}
        onSaved={onSaved}
        initialConnection={initialConnection}
      />
      <Button onClick={onConnections}>Manage connected accounts</Button>
    </>
  );
}
