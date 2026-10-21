import { ProviderIcon as SharedProviderIcon } from "@colossus/ui/components/ProviderIcon";
import type { ComponentProps } from "react";
import { safeSetupIcon } from "../setupPackages";
import { useSetupPresentation } from "../SetupPresentation";
export function ProviderIcon(props: ComponentProps<typeof SharedProviderIcon>) {
  const presentation = useSetupPresentation(props.provider);
  return (
    <SharedProviderIcon
      {...props}
      customIcon={safeSetupIcon(
        props.customIcon === undefined ? presentation?.icon : props.customIcon,
      )}
      customDarkIcon={safeSetupIcon(
        props.customDarkIcon === undefined
          ? presentation?.darkIcon
          : props.customDarkIcon,
      )}
    />
  );
}
