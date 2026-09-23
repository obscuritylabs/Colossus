import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { McpHealthDetails } from "./McpHealthDetails";
import type { ManagedMcpDiagnostic } from "../types";

describe("MCP health details", () => {
  it("keeps a failed TLS check and the worker's effective CA evidence visible", () => {
    const diagnostic: ManagedMcpDiagnostic = {
      server: "github",
      healthy: false,
      tools: [],
      message: "TLS verification failed. Check the imported PEM CA bundle.",
      report: {
        healthy: false,
        elapsedMs: 87,
        stage: "initialize",
        failure: { code: "tls", httpStatus: null },
        configuration: {
          runtimeVersion: "1.2.3",
          transport: "streamable_http",
          endpointSha256: "abc",
          additionalCaCertificates: 2,
          additionalCaSha256: "def",
          directHttp: true,
          credentialHeaders: 1,
          oauth: false,
          allowStateless: false,
          configuredTimeoutMs: 1000,
        },
      },
    };
    const html = renderToStaticMarkup(
      createElement(McpHealthDetails, { diagnostic }),
    );
    expect(html).toContain('role="alert"');
    expect(html).toContain("imported PEM");
    expect(html).toContain("2 additional CA certificates loaded");
    expect(html).toContain("colossus mcp doctor");
    expect(html).toContain("Export diagnostics");
    expect(html).not.toContain("No allowlisted tools");
    expect(html).not.toContain("allowlisted tools discovered.");
  });

  it("renders success without a failure alert", () => {
    const html = renderToStaticMarkup(
      createElement(McpHealthDetails, {
        diagnostic: { server: "github", healthy: true, tools: [] },
      }),
    );
    expect(html).toContain('role="status"');
    expect(html).toContain("No allowlisted tools");
    expect(html).toContain("0 allowlisted tools discovered.");
    expect(html).not.toContain('role="alert"');
  });

  it("keeps the discovered tool count alongside a native success message", () => {
    const html = renderToStaticMarkup(
      createElement(McpHealthDetails, {
        diagnostic: {
          server: "github",
          healthy: true,
          message: "MCP health check succeeded.",
          tools: [
            {
              server: "github",
              name: "list_issues",
              title: null,
              description: null,
            },
          ],
        },
      }),
    );
    expect(html).toContain("MCP health check succeeded.");
    expect(html).toContain("1 allowlisted tools discovered.");
    expect(html).toContain("list_issues");
    expect(html).not.toContain("No allowlisted tools");
  });
});
