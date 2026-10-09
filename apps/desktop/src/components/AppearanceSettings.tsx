import { AppearanceSettings as SharedAppearanceSettings } from "@colossus/ui/appearance";
import { useAppearance } from "../theme/AppearanceProvider";
import "@colossus/ui/styles/appearance.css";
export function AppearanceSettings() {
  return (
    <SharedAppearanceSettings
      controls={useAppearance()}
      description="Choose how Colossus looks on this device. Changes apply immediately and stay local to this Desktop installation."
    />
  );
}
