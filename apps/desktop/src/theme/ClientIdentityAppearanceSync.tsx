import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect } from "react";
import { useAppearance } from "./AppearanceProvider";

/** Only the main document sends bounded display preferences to native review. */
export function ClientIdentityAppearanceSync() {
  const { colorTheme, darkPalette, textSize } = useAppearance();
  useEffect(() => {
    if (!isTauri()) return;
    void invoke("sync_client_identity_appearance", {
      appearance: { colorTheme, darkPalette, textSize },
    }).catch(() => {
      // Appearance is best effort; it cannot authorize or delay a request.
    });
  }, [colorTheme, darkPalette, textSize]);
  return null;
}
