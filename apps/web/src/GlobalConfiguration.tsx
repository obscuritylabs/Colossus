import { useState } from "react";
import { DropdownSelect } from "@colossus/ui";
import type { Project } from "./control-api";
import type { ConnectionSettingsSection } from "./routes";
import { globalHref } from "./routes";
import { useNavigation } from "./navigation";
import { ChatSettings } from "./ChatSettings";
import { CredentialStoreSettings } from "./CredentialStoreSettings";

/** Instance authority is checked by the admin route and every server operation. */
export function GlobalConfiguration({
  section,
  identity,
  projects,
  selectedProject,
  onSaved,
}: {
  section: ConnectionSettingsSection;
  identity: string;
  projects: Project[];
  selectedProject: string;
  onSaved: () => void;
}) {
  const navigation = useNavigation();
  const [grantProject, setGrantProject] = useState(() =>
    projects.some((item) => item.id === selectedProject && !item.archived)
      ? selectedProject
      : "",
  );
  const availableProject =
    projects.find((item) => item.id === grantProject && !item.archived)?.id ??
    "";
  if (section === "credentials")
    return (
      <section aria-label="Instance secrets and assignments">
        <header className="catalog-heading">
          <div>
            <h2>Control Plane secrets</h2>
            <p>
              Save a secret once, then explicitly assign it to eligible project
              connections. Definition inheritance alone never grants credential
              access.
            </p>
          </div>
        </header>
        <label className="chat-setting-field">
          <span>Project for credential assignments</span>
          <DropdownSelect
            aria-label="Project for credential assignments"
            value={availableProject}
            onChange={(event) => setGrantProject(event.target.value)}
          >
            <option value="">Secrets only — choose a project to assign</option>
            {projects
              .filter((item) => !item.archived)
              .map((item) => (
                <option key={item.id} value={item.id}>
                  {item.name}
                </option>
              ))}
          </DropdownSelect>
        </label>
        <p className="field-help">
          Connections shown belong to the selected project and must use
          explicitly shared authentication. Rotating or deleting an instance
          secret affects every project assigned to it. Personal authentication
          always uses that person's private credentials.
        </p>
        <CredentialStoreSettings
          key={`${identity}:${availableProject}`}
          path="/api/admin/credential-store"
          resourcePath={`/api/admin/credential-store${availableProject ? `?project=${encodeURIComponent(availableProject)}` : ""}`}
          identity={identity}
          canEdit
          globalScope
          onSaved={onSaved}
        />
      </section>
    );
  return (
    <ChatSettings
      key={identity}
      project={selectedProject}
      identity={identity}
      canAdminister
      instanceScope
      settingsPath="/api/admin/hosted-settings"
      section={section}
      onClose={() => navigation.go(globalHref("admin", undefined, "users"))}
      onSaved={onSaved}
      onCredentials={() =>
        navigation.go(globalHref("admin", undefined, "credentials"))
      }
    />
  );
}
