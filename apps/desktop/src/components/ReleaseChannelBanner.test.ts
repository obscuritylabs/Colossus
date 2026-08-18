import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DesktopReleaseChannel, DesktopReleaseMetadata } from "../types";
import { ReleaseChannelBanner } from "./ReleaseChannelBanner";

function metadata(
  overrides: Partial<DesktopReleaseMetadata> = {},
): DesktopReleaseMetadata {
  return {
    platform: "windows",
    architecture: "x64",
    channel: "developer_preview",
    bundleIntegrity: "verified",
    codeSigning: "unsigned",
    ...overrides,
  };
}

function render(
  releaseChannel: DesktopReleaseChannel,
  releaseMetadata: DesktopReleaseMetadata | null = metadata({
    channel: releaseChannel,
  }),
): string {
  return renderToStaticMarkup(
    createElement(ReleaseChannelBanner, { releaseChannel, releaseMetadata }),
  );
}

describe("ReleaseChannelBanner", () => {
  it("clearly labels the unsigned Windows developer preview", () => {
    const markup = render("developer_preview", metadata());

    expect(markup).toContain("Developer Preview");
    expect(markup).toContain("Unsigned preview build for local testing");
  });

  it("preserves the macOS ad-hoc signing label", () => {
    const markup = render(
      "developer_preview",
      metadata({ platform: "macos", codeSigning: "ad_hoc" }),
    );

    expect(markup).toContain("Developer Preview");
    expect(markup).toContain("Ad-hoc signed and not Apple-notarized");
    expect(markup).not.toContain("Unsigned preview build");
  });

  it.each(["development", "stable", "validation_only"] as const)(
    "does not mislabel the %s channel as a developer preview",
    (releaseChannel) => {
      expect(render(releaseChannel)).toBe("");
    },
  );
});
