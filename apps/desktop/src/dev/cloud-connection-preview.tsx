import { CloudConnectionPane } from "../components/CloudConnectionPane";

/** Visual fixture only: never replaces a native bridge or proves enrollment. */
export default function CloudPreview() {
  const host = window as unknown as { __TAURI_INTERNALS__?: unknown };
  const parameters = new URLSearchParams(location.search);
  if (
    import.meta.env.DEV &&
    parameters.get("fixture") === "cloud" &&
    !host.__TAURI_INTERNALS__
  ) {
    let enrolled = parameters.get("state") !== "new";
    let status = parameters.get("state") ?? "connected";
    const snapshot = () => ({
      targetId: "preview-workspace",
      status,
      nodeId: enrolled ? "01a10a7f40fb779191d88fe3e4f2f1b6" : null,
      projectId: enrolled ? "Example project" : null,
      endpoint: enrolled ? "https://cloud.example.com:443" : null,
    });
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        switch (command) {
          case "cloud_status":
            break;
          case "cloud_enroll":
            enrolled = true;
            status = "connected";
            break;
          case "cloud_connect":
            status = "connected";
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
    <main style={{ maxWidth: 850, margin: "32px auto", padding: "0 24px" }}>
      <p
        style={{
          color: "var(--muted)",
          fontSize: "var(--font-size-caption)",
        }}
      >
        Desktop visual preview · Native enrollment is tested separately.
      </p>
      <CloudConnectionPane targetId="preview-workspace" />
    </main>
  );
}
