import type { CredentialTarget } from "./credential-types";
import type { PublicSettings } from "./control-api";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";
import { Button, SettingsFrame } from "@colossus/ui";
import { AppearanceSettings } from "./Appearance";
import { ChatSettings } from "./ChatSettings";
import { ChatCredentials } from "./ChatCredentials";
import { ChatAccounts } from "./ChatAccounts";
import { PersonalVariables } from "./PersonalVariables";
import { IconShield } from "@tabler/icons-react";
import type {
  ConnectionSettingsSection,
  PersonalSettingsSection,
} from "./routes";
import "./chat-workspace.css";

const projectTabs = [
  { id: "providers", label: "Providers", group: "Project connections" },
  { id: "models", label: "Models", group: "Project connections" },
  { id: "mcp", label: "MCP connections", group: "Project connections" },
  { id: "routing", label: "Role routing", group: "Project connections" },
  { id: "search", label: "Search", group: "Project services" },
  { id: "telemetry", label: "Telemetry", group: "Project services" },
  { id: "credentials", label: "Secrets", group: "Secrets and variables" },
  { id: "variables", label: "Variables", group: "Secrets and variables" },
] as const;
const personalTabs = [
  { id: "appearance", label: "Appearance", group: "Preferences" },
  { id: "account", label: "Account", group: "Preferences" },
  { id: "credentials", label: "Credentials", group: "Connections" },
  { id: "variables", label: "Variables", group: "Connections" },
  { id: "connections", label: "Connected accounts", group: "Connections" },
] as const;

/** Desktop settings geometry with browser preferences and project authority kept separate. */
export function WebSettings({
  project,
  projectName,
  identity,
  displayName,
  email,
  canAdminister,
  canConnect,
  appearance,
  onAppearance,
  isGlobalAdmin = false,
  initialScope = "global",
  initialSection,
  onSectionChange,
  onInstanceSettings,
  onReturn,
  onConnections,
  onSaved,
}: {
  project: string;
  projectName: string;
  identity: string;
  displayName: string;
  email: string | null;
  canAdminister: boolean;
  canConnect: boolean;
  appearance: ComponentProps<typeof AppearanceSettings>["appearance"];
  onAppearance: ComponentProps<typeof AppearanceSettings>["onChange"];
  isGlobalAdmin?: boolean;
  onEnvironmentSettings?: (settings: PublicSettings) => void;
  initialScope?: "global" | "space";
  initialSection?:
    ConnectionSettingsSection | PersonalSettingsSection | undefined;
  onSectionChange?: (scope: "global" | "space", section: string) => void;
  onInstanceSettings?: () => void;
  onReturn: () => void;
  onConnections: () => void;
  onSaved: () => void;
}) {
  const [scope, setScope] = useState(initialScope);
  useEffect(() => setScope(initialScope), [initialScope]);
  const [query, setQuery] = useState("");
  const [personalTab, setPersonalTab] = useState("appearance");
  const [projectCredential, setProjectCredential] = useState<{
    project: string;
    connection: CredentialTarget | undefined;
    request: number;
  } | null>(null);
  const [personalCredential, setPersonalCredential] = useState<{
    project: string;
    connection: CredentialTarget | undefined;
    request: number;
  } | null>(null);
  const [projectTab, setProjectTab] =
    useState<ConnectionSettingsSection>("providers");
  useEffect(() => {
    if (!initialSection) return;
    if (
      initialScope === "space" &&
      projectTabs.some((tab) => tab.id === initialSection)
    )
      setProjectTab(initialSection as ConnectionSettingsSection);
    else if (
      initialScope === "global" &&
      personalTabs.some((tab) => tab.id === initialSection)
    )
      setPersonalTab(initialSection);
  }, [initialScope, initialSection]);
  const tabs = scope === "global" ? personalTabs : projectTabs;
  const frame = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const content = frame.current?.querySelector(".settings-main");
    if (content) {
      content.scrollTop = 0;
      content.scrollLeft = 0;
    }
  }, [scope, personalTab, projectTab, project]);
  function selectTab(id: string) {
    setQuery("");
    if (scope === "global") setPersonalTab(id);
    else setProjectTab(id as typeof projectTab);
    onSectionChange?.(scope, id);
  }
  return (
    <div className="web-settings-window" ref={frame}>
      <SettingsFrame
        scope={scope}
        onScopeChange={(value) => {
          setScope(value);
          setQuery("");
          onSectionChange?.(
            value,
            value === "global" ? personalTab : projectTab,
          );
        }}
        globalLabel="Personal"
        workspaceLabel="Project"
        showWorkspaceScope={canAdminister}
        globalDescription="Personal preferences and connections"
        workspaceDescription="Project definitions and explicit shared credentials"
        additionalScopes={
          isGlobalAdmin && onInstanceSettings ? (
            <button
              type="button"
              className="sidebar-nav-item"
              onClick={onInstanceSettings}
            >
              <IconShield size={17} aria-hidden="true" />
              <span>
                <strong>Control Plane Admin</strong>
                <small>Instance catalogs and administration</small>
              </span>
            </button>
          ) : null
        }
        query={query}
        onQueryChange={(value) => {
          setQuery(value);
          const match = tabs.find((tab) =>
            tab.label.toLowerCase().includes(value.trim().toLowerCase()),
          );
          if (match) {
            if (scope === "global") setPersonalTab(match.id);
            else setProjectTab(match.id as typeof projectTab);
          }
        }}
        tabs={tabs}
        activeTab={scope === "global" ? personalTab : projectTab}
        onTabChange={selectTab}
        workspaceContext={
          <div className="space-settings-context">
            <strong>{projectName || "No project selected"}</strong>
            <span className="field-help">
              Project-managed providers and tools. Chat history and personal
              credentials stay private.
            </span>
          </div>
        }
        onReturnToWork={onReturn}
      >
        <div hidden={scope !== "global"} className="web-personal-settings">
          {!project &&
          ["credentials", "connections", "variables"].includes(personalTab) ? (
            <section className="control-card">
              <h2>Choose a project</h2>
              <p>
                Your connections, secrets, and variables stay private to your
                account in the selected project.
              </p>
            </section>
          ) : personalTab === "appearance" ? (
            <AppearanceSettings
              appearance={appearance}
              onChange={onAppearance}
              isGlobalAdmin={false}
            />
          ) : personalTab === "variables" ? (
            <PersonalVariables
              key={`${project}:${identity}`}
              project={project}
              identity={identity}
              canEdit={canConnect}
            />
          ) : personalTab === "credentials" ? (
            <ChatCredentials
              key={`${project}:${identity}:${personalCredential?.project === project ? personalCredential.request : 0}`}
              project={project}
              identity={identity}
              canConnect={canConnect}
              onConnections={() => {
                setPersonalTab("connections");
                onSectionChange?.("global", "connections");
              }}
              onSaved={onSaved}
              initialConnection={
                personalCredential?.project === project
                  ? personalCredential.connection
                  : undefined
              }
            />
          ) : personalTab === "connections" ? (
            <ChatAccounts
              key={`${project}:${identity}`}
              project={project}
              identity={identity}
              canConnect={canConnect}
            />
          ) : (
            <section className="control-card" aria-label="Signed-in account">
              <header>
                <h2>Account</h2>
              </header>
              <p>{displayName}</p>
              <p className="field-help">{email ?? ""}</p>
              <Button onClick={onConnections}>Manage your connections</Button>
            </section>
          )}
        </div>
        <div hidden={scope !== "space"}>
          {canAdminister ? (
            <ChatSettings
              key={`${project}:${identity}:${projectCredential?.project === project ? projectCredential.request : 0}`}
              project={project}
              identity={identity}
              canAdminister
              onClose={onReturn}
              onSaved={onSaved}
              section={projectTab}
              credentialTarget={
                projectCredential?.project === project
                  ? projectCredential.connection
                  : undefined
              }
              onCredentials={(connection) => {
                setProjectCredential((current) => ({
                  project,
                  connection,
                  request: (current?.request ?? 0) + 1,
                }));
                setQuery("");
                setProjectTab("credentials");
                onSectionChange?.("space", "credentials");
              }}
              onPersonalCredentials={(connection) => {
                setPersonalCredential((current) => ({
                  project,
                  connection,
                  request: (current?.request ?? 0) + 1,
                }));
                setQuery("");
                setScope("global");
                setPersonalTab("credentials");
                onSectionChange?.("global", "credentials");
              }}
            />
          ) : (
            <section className="web-personal-settings">
              <h2>Project connections</h2>
              <p>
                Providers, models and tools are managed by your project
                administrator.
              </p>
              <Button onClick={onConnections}>Manage your connections</Button>
            </section>
          )}
        </div>
      </SettingsFrame>
    </div>
  );
}
