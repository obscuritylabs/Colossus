import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  ExecutionBoundaryBanner,
  executionBoundaryBannerVisible,
  managedRuntimeBoundaryActive,
} from "./ExecutionBoundaryBanner";

describe("ExecutionBoundaryBanner", () => {
  it("persistently exposes the unsafe Managed Local runtime boundary", () => {
    const markup = renderToStaticMarkup(
      createElement(ExecutionBoundaryBanner, {
        active: true,
        boundary: "full_access",
      }),
    );

    expect(markup).toContain('role="alert"');
    expect(markup).toContain(
      'aria-label="Unsafe Managed Local execution boundary"',
    );
    expect(markup).toContain("Unsafe: Full access");
    expect(markup).toContain("without Colossus isolation");
    expect(markup).toContain("Approval mode is separate");
  });

  it("does not warn for isolated or inactive runtimes", () => {
    for (const props of [
      { active: true, boundary: "workspace_isolated" as const },
      { active: true, boundary: "offline_isolated" as const },
      { active: false, boundary: "full_access" as const },
    ]) {
      expect(
        renderToStaticMarkup(createElement(ExecutionBoundaryBanner, props)),
      ).toBe("");
    }
  });

  it("stays active for a running Managed Local runtime independent of target selection", () => {
    for (const state of [
      "starting",
      "ready",
      "restarting",
      "stopping",
    ] as const) {
      expect(managedRuntimeBoundaryActive(state)).toBe(true);
    }
    for (const state of [
      "needs_workspace",
      "needs_provider",
      "failed",
    ] as const) {
      expect(managedRuntimeBoundaryActive(state)).toBe(false);
    }
  });

  it("reserves the shell warning only for an active Full access runtime", () => {
    expect(executionBoundaryBannerVisible("ready", "full_access")).toBe(true);
    expect(executionBoundaryBannerVisible("ready", "workspace_isolated")).toBe(
      false,
    );
    expect(executionBoundaryBannerVisible("failed", "full_access")).toBe(false);
  });
});
