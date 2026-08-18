import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DesktopReleaseChannel } from "../types";
import { ReleaseChannelBanner } from "./ReleaseChannelBanner";

function render(releaseChannel: DesktopReleaseChannel): string {
  return renderToStaticMarkup(
    createElement(ReleaseChannelBanner, { releaseChannel }),
  );
}

describe("ReleaseChannelBanner", () => {
  it("clearly labels the unsigned developer preview", () => {
    const markup = render("developer_preview");

    expect(markup).toContain("Developer Preview");
    expect(markup).toContain("Unsigned preview build for local testing");
  });

  it.each(["development", "stable", "validation_only"] as const)(
    "does not mislabel the %s channel as a developer preview",
    (releaseChannel) => {
      expect(render(releaseChannel)).toBe("");
    },
  );
});
