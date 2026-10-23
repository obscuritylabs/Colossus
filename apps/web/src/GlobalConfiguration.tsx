import { useState } from "react";
import { DropdownSelect } from "@colossus/ui";
import type { Project } from "./control-api";
import type { ConnectionSettingsSection } from "./routes";
import { globalHref } from "./routes";
import { useNavigation } from "./navigation";
import { ChatSettings } from "./ChatSettings";
import { CredentialStoreSettings } from "./CredentialStoreSettings";
import type { SettingsEditorNavigation } from "./settings-editor";

/** Instance authority is checked by the admin route and every server operation. */
export function GlobalConfiguration({
  section,
  identity,
  projects,
  selectedProject,
  onSaved,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
  onProjectChange,
}: SettingsEditorNavigation & {
  section: ConnectionSettingsSection;
  identity: string;
  projects: Project[];
  selectedProject: string;
  onSaved: () => void;
  onProjectChange?: ((id: string) => void) | undefined;
}) {
  const navigation = useNavigation();
  const [grantProject, setGrantProject] = useState(() =>
    projects.some((item) => item.id === selectedProject && !item.archived)
      ? selectedProject
      : "",
  );
  const effectiveProject = onProjectChange ? selectedProject : grantProject;
  const availableProject =
    projects.find((item) => item.id === effectiveProject && !item.archived)
      ?.id ?? "";
  if (section === "credentials")
    return (
      <section aria-label="Instance secrets and assignments">
        <CredentialStoreSettings
          key={`${identity}:${availableProject}`}
          path="/api/admin/credential-store"
          resourcePath={`/api/admin/credential-store${availableProject ? `?project=${encodeURIComponent(availableProject)}` : ""}`}
          identity={identity}
          canEdit
          globalScope
          assignmentContext={
            <>
              <label className="chat-setting-field">
                <span>Project for credential assignments</span>
                <DropdownSelect
                  aria-label="Project for credential assignments"
                  value={availableProject}
                  onChange={(event) =>
                    onProjectChange
                      ? onProjectChange(event.target.value)
                      : setGrantProject(event.target.value)
                  }
                >
                  <option value="">
                    Secrets only — choose a project to assign
                  </option>
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
                Assignments apply to the selected project's shared connections.
                Rotating or deleting an instance secret affects every project
                assigned to it.
              </p>
            </>
          }
          editorLocation={editorLocation}
          onOpenEditor={onOpenEditor}
          onCloseEditor={onCloseEditor}
          onDirtyChange={onDirtyChange}
          onSaved={onSaved}
        />
      </section>
    );
  return (
    <ChatSettings
      key={`${identity}:${section}:${JSON.stringify(editorLocation)}`}
      project={selectedProject}
      identity={identity}
      canAdminister
      instanceScope
      settingsPath="/api/admin/hosted-settings"
      section={section}
      editorLocation={editorLocation}
      onOpenEditor={onOpenEditor}
      onCloseEditor={onCloseEditor}
      onDirtyChange={onDirtyChange}
      onClose={() => navigation.go(globalHref("admin", undefined, "users"))}
      onSaved={onSaved}
      onCredentials={() =>
        navigation.go(globalHref("admin", undefined, "credentials"))
      }
    />
  );
}
