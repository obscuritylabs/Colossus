import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PluginMcpControls } from "./PluginMcpControls";

describe("plugin MCP controls", () => {
  it("offers explicit enablement while keeping diagnostics disabled", () => {
    const markup = renderToStaticMarkup(
      createElement(PluginMcpControls, {
        spaceId: "workspace",
        server: "example/docs",
        enabled: false,
        pluginActive: true,
        http: true,
      }),
    );
    expect(markup).toContain("Enable all plugin tools");
    expect(markup).toContain("including tools");
    expect(markup.match(/disabled=""/g)).toHaveLength(2);
    expect(markup).not.toContain("Sign in");
  });
  it("does not offer HTTP OAuth controls for a stdio server", () => {
    const markup = renderToStaticMarkup(
      createElement(PluginMcpControls, {
        spaceId: "workspace",
        server: "example/local",
        enabled: true,
        pluginActive: true,
        http: false,
      }),
    );
    expect(markup).toContain("Test connection");
    expect(markup).not.toContain("OAuth status");
  });
});
