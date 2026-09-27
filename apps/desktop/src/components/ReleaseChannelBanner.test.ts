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
    codeSigning: "verified",
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
  it("identifies the signed Windows developer preview", () => {
    const markup = render("developer_preview", metadata());

    expect(markup).toContain("Developer Preview");
    expect(markup).toContain("Signed by Obscurity Labs LLC");
  });

  it("preserves the macOS ad-hoc signing label", () => {
    const markup = render(
      "developer_preview",
      metadata({ platform: "macos", codeSigning: "ad_hoc" }),
    );

    expect(markup).toContain("Developer Preview");
    expect(markup).toContain("Ad-hoc signed and not Apple-notarized");
  });

  it.each(["development", "stable", "validation_only"] as const)(
    "does not mislabel the %s channel as a developer preview",
    (releaseChannel) => {
      expect(render(releaseChannel)).toBe("");
    },
  );
});
