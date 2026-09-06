import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PluginMcpControls } from "./PluginMcpControls";

describe("plugin MCP controls", () => {
  it("keeps all connection actions disabled until runtime enablement", () => {
    const markup = renderToStaticMarkup(
      createElement(PluginMcpControls, {
        targetId: "workspace",
        server: "example/docs",
        enabled: false,
        http: true,
      }),
    );
    expect(markup).toContain("Enable and apply this server explicitly");
    expect(markup.match(/disabled=""/g)).toHaveLength(2);
    expect(markup).not.toContain("Sign in");
  });
  it("does not offer HTTP OAuth controls for a stdio server", () => {
    const markup = renderToStaticMarkup(
      createElement(PluginMcpControls, {
        targetId: "workspace",
        server: "example/local",
        enabled: true,
        http: false,
      }),
    );
    expect(markup).toContain("Test connection");
    expect(markup).not.toContain("OAuth status");
  });
});
