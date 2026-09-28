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

  let description = "Preview build for local testing";
  if (
    releaseMetadata?.platform === "macos" &&
    releaseMetadata.codeSigning === "ad_hoc"
  ) {
    description = "Ad-hoc signed and not Apple-notarized";
  } else if (
    releaseMetadata?.platform === "windows" &&
    releaseMetadata.codeSigning === "verified"
  ) {
    description = "Signed by Obscurity Labs LLC; preview build for testing";
  }

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
