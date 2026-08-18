import type { DesktopReleaseChannel, DesktopReleaseMetadata } from "../types";

interface ReleaseChannelBannerProps {
  releaseChannel: DesktopReleaseChannel;
  releaseMetadata: DesktopReleaseMetadata | null;
}

export function ReleaseChannelBanner({
  releaseChannel,
  releaseMetadata,
}: ReleaseChannelBannerProps) {
  if (releaseChannel !== "developer_preview") {
    return null;
  }

  const description =
    releaseMetadata?.platform === "macos" &&
    releaseMetadata.codeSigning === "ad_hoc"
      ? "Ad-hoc signed and not Apple-notarized"
      : releaseMetadata?.platform === "windows" &&
          releaseMetadata.codeSigning === "unsigned"
        ? "Unsigned preview build for local testing"
        : "Preview build for local testing";

  return (
    <aside
      className="release-channel-banner"
      aria-label="Colossus Developer Preview build"
    >
      <strong>Developer Preview</strong>
      <span aria-hidden="true">•</span>
      <span>{description}</span>
    </aside>
  );
}
