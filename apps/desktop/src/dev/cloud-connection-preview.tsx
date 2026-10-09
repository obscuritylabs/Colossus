import { CloudConnectionPane } from "../components/CloudConnectionPane";
import {
  ControlPlaneSettingsPane,
  type ControlPlaneProfiles,
} from "../components/ControlPlaneSettingsPane";
import { SettingsFrame } from "@colossus/ui";
import { useState } from "react";

/** Visual fixture only: never replaces a native bridge or proves enrollment. */
export default function CloudPreview() {
  const host = window as unknown as { __TAURI_INTERNALS__?: unknown };
  const parameters = new URLSearchParams(location.search);
  const [scope, setScope] = useState<"global" | "space">(
    parameters.get("pane") === "global" ? "global" : "space",
  );
  if (
    import.meta.env.DEV &&
    parameters.get("fixture") === "cloud" &&
    !host.__TAURI_INTERNALS__
  ) {
    let enrolled = parameters.get("state") !== "new";
    let status = parameters.get("state") ?? "connected";
    let sharedSessions = false;
    let sharedContinuation = false;
    let sharingRecoveryRequired = parameters.get("recovery") === "1";
    const sharingRestartRequired = parameters.get("restart") === "1";
    let catalog: ControlPlaneProfiles = {
      revision: 0,
      profiles: [],
      defaultProfile: null,
      connections: enrolled
        ? [
            {
              targetId: "preview-workspace",
              status,
              projectId: "Example project",
              endpoint: "https://control-plane.example.com",
              sharedSessions: false,
              sharingRecoveryRequired,
              sharingRestartRequired,
            },
          ]
        : [],
    };
    const snapshot = () => ({
      targetId: "preview-workspace",
      status,
      nodeId: enrolled ? "01a10a7f40fb779191d88fe3e4f2f1b6" : null,
      projectId: enrolled ? "Example project" : null,
      endpoint: enrolled ? "https://cloud.example.com:443" : null,
      hostId: enrolled ? "host-preview" : null,
      workspaceId: "workspace-preview",
      sharingSupported: true,
      sharedSessions,
      sharedContinuation,
      sharingRecoveryRequired,
      sharingRestartRequired,
    });
    host.__TAURI_INTERNALS__ = {
      invoke: async (
        command: string,
        arguments_: {
          enabled?: boolean;
          allowContinuation?: boolean;
          catalog?: ControlPlaneProfiles;
        } = {},
      ) => {
        switch (command) {
          case "control_plane_profiles":
            return catalog;
          case "save_control_plane_profiles":
            if (!arguments_.catalog) throw new Error("Missing visual catalog");
            catalog = {
              ...arguments_.catalog,
              revision: catalog.revision + 1,
              connections: catalog.connections,
            };
            return catalog;
          case "cloud_status":
            break;
          case "cloud_set_workspace_sharing":
            sharedSessions = arguments_.enabled ?? false;
            sharedContinuation = arguments_.allowContinuation ?? false;
            sharingRecoveryRequired = false;
            break;
          case "cloud_enroll":
            enrolled = true;
            status = "connected";
            break;
          case "cloud_connect":
            status = "connected";
            sharingRecoveryRequired = false;
            break;
          case "cloud_disconnect":
            status = "disconnected";
            break;
          case "cloud_revoke":
            status = "revoked";
            break;
          case "cloud_forget":
            enrolled = false;
            status = "disconnected";
            break;
          default:
            throw new Error("Unsupported visual fixture command");
        }
        return snapshot();
      },
    };
    if (!enrolled) status = "disconnected";
  }
  return (
    <div className="app-shell app-shell--settings">
      <div className="settings-scroll">
        <SettingsFrame
          scope={scope}
          onScopeChange={setScope}
          query=""
          onQueryChange={() => undefined}
          tabs={[{ id: "control-plane", label: "Control Plane" }]}
          activeTab="control-plane"
          onTabChange={() => undefined}
          workspaceContext={
            <div className="space-settings-context">
              <strong>Preview workspace</strong>
              <small>Sample connection</small>
            </div>
          }
        >
          <div className="settings-page-context">
            <p className="muted">
              Desktop visual preview · Native enrollment is tested separately.
            </p>
          </div>
          {scope === "global" ? (
            <ControlPlaneSettingsPane />
          ) : (
            <CloudConnectionPane targetId="preview-workspace" />
          )}
        </SettingsFrame>
      </div>
    </div>
  );
}
