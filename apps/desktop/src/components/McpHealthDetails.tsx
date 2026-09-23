import type { ManagedMcpDiagnostic } from "../types";

export function McpHealthDetails({
  diagnostic,
}: {
  diagnostic: ManagedMcpDiagnostic;
}) {
  const report = diagnostic.report;
  return (
    <div className="mcp-diagnostic-detail">
      <p role={diagnostic.healthy ? "status" : "alert"}>
        {diagnostic.message ??
          (diagnostic.healthy
            ? `${diagnostic.tools.length} tools discovered.`
            : "MCP health check failed.")}
      </p>
      {diagnostic.healthy && (
        <span>
          {diagnostic.tools.map((tool) => tool.name).join(", ") ||
            "No allowlisted tools"}
        </span>
      )}
      {report && (
        <details>
          <summary>Connection diagnostics · {report.elapsedMs} ms</summary>
          <p>Stage: {report.stage.replaceAll("_", " ")}</p>
          {report.configuration && (
            <p>
              Runtime {report.configuration.runtimeVersion} ·{" "}
              {report.configuration.transport === "stdio"
                ? "The MCP subprocess manages its own TLS trust."
                : `${report.configuration.additionalCaCertificates} additional CA certificates loaded; direct HTTP connection.`}
            </p>
          )}
          <p>
            Compare this report with{" "}
            <code>colossus mcp doctor &lt;server&gt;</code> on this computer.
            Recent checks are included in Settings → Diagnostics → Export
            diagnostics.
          </p>
          <pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
            {JSON.stringify(report, null, 2)}
          </pre>
        </details>
      )}
    </div>
  );
}
