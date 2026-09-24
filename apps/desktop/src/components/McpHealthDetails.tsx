import { IconAlertTriangle, IconCircleCheck } from "@tabler/icons-react";
import type { ManagedMcpDiagnostic } from "../types";

export function McpHealthDetails({
  diagnostic,
}: {
  diagnostic: ManagedMcpDiagnostic;
}) {
  const report = diagnostic.report;
  const configuration = report?.configuration;
  const StatusIcon = diagnostic.healthy ? IconCircleCheck : IconAlertTriangle;
  return (
    <div className="mcp-diagnostic-detail">
      <div className="mcp-health-summary">
        <StatusIcon
          size={20}
          className={diagnostic.healthy ? "is-healthy" : "is-failed"}
          aria-hidden="true"
        />
        <div>
          <strong>
            {diagnostic.healthy ? "Connection healthy" : "Connection failed"}
          </strong>
          <p role={diagnostic.healthy ? "status" : "alert"}>
            {diagnostic.message ??
              (diagnostic.healthy
                ? "MCP connection and tool discovery succeeded."
                : "MCP health check failed.")}
          </p>
          {diagnostic.healthy && (
            <p>{diagnostic.tools.length} allowlisted tools discovered.</p>
          )}
        </div>
        {report && (
          <span className="mcp-health-duration">{report.elapsedMs} ms</span>
        )}
      </div>
      {report && (
        <dl className="mcp-health-facts">
          <div>
            <dt>Stage</dt>
            <dd>{report.stage.replaceAll("_", " ")}</dd>
          </div>
          {configuration && (
            <>
              <div>
                <dt>Runtime</dt>
                <dd>{configuration.runtimeVersion}</dd>
              </div>
              <div>
                <dt>Connection</dt>
                <dd>
                  {configuration.transport === "stdio"
                    ? "Local process (stdio)"
                    : configuration.directHttp
                      ? "Direct HTTP"
                      : "HTTP"}
                </dd>
              </div>
              <div>
                <dt>TLS trust</dt>
                <dd>
                  {configuration.transport === "stdio"
                    ? "The MCP subprocess manages its own TLS trust."
                    : `${configuration.additionalCaCertificates} additional CA certificates loaded`}
                </dd>
              </div>
            </>
          )}
          {report.failure && (
            <div>
              <dt>Failure</dt>
              <dd>
                {report.failure.code.replaceAll("_", " ")}
                {report.failure.httpStatus !== null &&
                  ` · HTTP ${report.failure.httpStatus}`}
              </dd>
            </div>
          )}
        </dl>
      )}
      {diagnostic.healthy &&
        (diagnostic.tools.length > 0 ? (
          <details className="mcp-health-disclosure">
            <summary>Discovered tools</summary>
            <ul
              className="mcp-health-tools"
              aria-label={`${diagnostic.server} discovered tools`}
              tabIndex={0}
            >
              {diagnostic.tools.map((tool) => (
                <li key={tool.name}>
                  <code>{tool.name}</code>
                </li>
              ))}
            </ul>
          </details>
        ) : (
          <p className="mcp-health-empty">No allowlisted tools</p>
        ))}
      {report && (
        <details className="mcp-health-disclosure">
          <summary>Connection diagnostics</summary>
          <div className="mcp-health-report">
            <p>
              Compare this report with{" "}
              <code>colossus mcp doctor &lt;server&gt;</code> on this computer.
              Recent checks are included in Settings → Global → Desktop → Export
              diagnostics.
            </p>
            <pre
              tabIndex={0}
              aria-label={`${diagnostic.server} connection diagnostic report`}
            >
              {JSON.stringify(report, null, 2)}
            </pre>
          </div>
        </details>
      )}
    </div>
  );
}
