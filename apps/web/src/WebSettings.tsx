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
import "./chat-workspace.css";

const projectTabs = [
  { id: "providers", label: "Providers", group: "Project connections" },
  { id: "models", label: "Models", group: "Project connections" },
  { id: "mcp", label: "MCP connections", group: "Project connections" },
  { id: "routing", label: "Role routing", group: "Project connections" },
  { id: "credentials", label: "Credentials", group: "Project connections" },
] as const;
const personalTabs = [
  { id: "appearance", label: "Appearance", group: "Preferences" },
  { id: "account", label: "Account", group: "Preferences" },
  { id: "credentials", label: "Credentials", group: "Connections" },
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
  initialScope = "global",
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
  initialScope?: "global" | "space";
  onReturn: () => void;
  onConnections: () => void;
  onSaved: () => void;
}) {
  const [scope, setScope] = useState(initialScope);
  useEffect(() => setScope(initialScope), [initialScope]);
  const [query, setQuery] = useState("");
  const [personalTab, setPersonalTab] = useState("appearance");
  const [projectTab, setProjectTab] = useState<
    "providers" | "models" | "mcp" | "credentials" | "routing"
  >("providers");
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
  }
  return (
    <div className="web-settings-window" ref={frame}>
      <SettingsFrame
        scope={scope}
        onScopeChange={(value) => {
          setScope(value);
          setQuery("");
        }}
        globalDescription="Personal preferences and connections"
        workspaceDescription="Shared project connections"
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
          {personalTab === "appearance" ? (
            <AppearanceSettings
              appearance={appearance}
              onChange={onAppearance}
            />
          ) : personalTab === "credentials" ? (
            <ChatCredentials
              key={`${project}:${identity}`}
              project={project}
              identity={identity}
              canConnect={canConnect}
              onConnections={onConnections}
              onSaved={onSaved}
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
              project={project}
              identity={identity}
              canAdminister
              onClose={onReturn}
              onSaved={onSaved}
              section={projectTab}
              onCredentials={() => {
                setQuery("");
                setProjectTab("credentials");
              }}
              onPersonalCredentials={() => {
                setQuery("");
                setScope("global");
                setPersonalTab("credentials");
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
