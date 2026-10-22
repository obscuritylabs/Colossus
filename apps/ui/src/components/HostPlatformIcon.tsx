import {
  IconBrandAndroid,
  IconBrandApple,
  IconBrandUbuntu,
  IconBrandWindows,
  IconDeviceDesktop,
} from "@tabler/icons-react";

/** Host-reported platform decoration. Labels and connection authority stay with the host. */
export function HostPlatformIcon({
  platform,
  size = 20,
}: {
  platform: string;
  size?: number;
}) {
  const icons: Record<string, typeof IconBrandWindows> = {
    windows: IconBrandWindows,
    macos: IconBrandApple,
    ios: IconBrandApple,
    linux: IconBrandUbuntu,
    android: IconBrandAndroid,
  };
  const Icon = icons[platform] ?? IconDeviceDesktop;
  return <Icon size={size} stroke={1.7} aria-hidden="true" />;
}
