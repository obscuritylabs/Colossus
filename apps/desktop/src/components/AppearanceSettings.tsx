import { IconPalette, IconTypography } from "@tabler/icons-react";

import type {
  ColorThemePreference,
  TextSizePreference,
} from "../theme/appearance";
import { useAppearance } from "../theme/AppearanceProvider";
import { DropdownSelect } from "./DropdownSelect";

const COLOR_THEME_COPY: Record<ColorThemePreference, string> = {
  system: "Match your operating system and update automatically.",
  dark: "Use the dark Colossus palette on this device.",
  light: "Use the light Colossus palette on this device.",
};

const TEXT_SIZE_COPY: Record<TextSizePreference, string> = {
  compact: "Fit more information on screen with smaller type.",
  comfortable: "Use the balanced default size for everyday work.",
  large: "Increase text and controls for easier reading.",
};

export function AppearanceSettings() {
  const {
    colorTheme,
    resolvedColorTheme,
    setColorTheme,
    setTextSize,
    textSize,
  } = useAppearance();

  return (
    <section
      className="appearance-settings-card"
      aria-labelledby="appearance-settings-heading"
    >
      <div className="desktop-panel-heading appearance-settings-heading">
        <div>
          <h4 id="appearance-settings-heading">Appearance</h4>
          <p>
            Choose how Colossus looks on this device. Changes apply immediately
            and stay local to this Desktop installation.
          </p>
        </div>
        <span className="status-chip tone-neutral" aria-live="polite">
          {resolvedColorTheme === "dark" ? "Dark palette" : "Light palette"}
        </span>
      </div>
      <div className="appearance-control-grid">
        <label htmlFor="appearance-color-theme">
          <span className="appearance-control-icon">
            <IconPalette size={18} aria-hidden="true" />
          </span>
          <span className="appearance-control-copy">
            <strong>Color theme</strong>
            <small id="appearance-color-theme-help">
              {COLOR_THEME_COPY[colorTheme]}
            </small>
          </span>
          <DropdownSelect
            id="appearance-color-theme"
            value={colorTheme}
            aria-describedby="appearance-color-theme-help"
            onChange={(event) =>
              setColorTheme(event.target.value as ColorThemePreference)
            }
          >
            <option value="system">System</option>
            <option value="dark">Dark</option>
            <option value="light">Light</option>
          </DropdownSelect>
        </label>
        <label htmlFor="appearance-text-size">
          <span className="appearance-control-icon">
            <IconTypography size={18} aria-hidden="true" />
          </span>
          <span className="appearance-control-copy">
            <strong>Text size</strong>
            <small id="appearance-text-size-help">
              {TEXT_SIZE_COPY[textSize]}
            </small>
          </span>
          <DropdownSelect
            id="appearance-text-size"
            value={textSize}
            aria-describedby="appearance-text-size-help"
            onChange={(event) =>
              setTextSize(event.target.value as TextSizePreference)
            }
          >
            <option value="compact">Compact</option>
            <option value="comfortable">Comfortable</option>
            <option value="large">Large</option>
          </DropdownSelect>
        </label>
      </div>
    </section>
  );
}
