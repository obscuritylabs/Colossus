import type { DesktopReleaseChannel } from "../types";

interface ReleaseChannelBannerProps {
  releaseChannel: DesktopReleaseChannel;
}

export function ReleaseChannelBanner({
  releaseChannel,
}: ReleaseChannelBannerProps) {
  if (releaseChannel !== "developer_preview") {
    return null;
  }

  return (
    <aside
      className="release-channel-banner"
      aria-label="Colossus Developer Preview build"
    >
      <strong>Developer Preview</strong>
      <span aria-hidden="true">•</span>
      <span>Ad-hoc signed and not Apple-notarized</span>
    </aside>
  );
}
