import { IconPalette, IconRefresh, IconTypography } from "@tabler/icons-react";
import { useState, type CSSProperties } from "react";

import { isDefaultPalette, palettePreviewVariables } from "../lib/palette";
import type { PaletteColor, PaletteTheme, ThemePalettes } from "../lib/palette";
import { DropdownSelect } from "./DropdownSelect";
type ColorThemePreference = "system" | "dark" | "light";
type DarkPalettePreference = "colossus" | "neutral" | "hacker";
type TextSizePreference = "compact" | "comfortable" | "large";
export interface AppearanceControls {
  colorTheme: ColorThemePreference;
  darkPalette: DarkPalettePreference;
  resolvedColorTheme: PaletteTheme;
  textSize: TextSizePreference;
  palettes: ThemePalettes;
  setColorTheme: (value: ColorThemePreference) => void;
  setDarkPalette: (value: DarkPalettePreference) => void;
  setTextSize: (value: TextSizePreference) => void;
  setPaletteColor: (
    theme: PaletteTheme,
    slot: PaletteColor,
    value: string,
  ) => boolean;
  resetPalette: (theme: PaletteTheme) => void;
  showSecurityWarnings?: boolean;
  setShowSecurityWarnings?: (value: boolean) => void;
}
const COLOR_THEME_COPY: Record<ColorThemePreference, string> = {
  system: "Match your operating system and update automatically.",
  dark: "Use dark mode on this device.",
  light: "Use light mode on this device.",
};

const TEXT_SIZE_COPY: Record<TextSizePreference, string> = {
  compact: "Fit more information on screen with smaller type.",
  comfortable: "Use the balanced default size for everyday work.",
  large: "Increase text and controls for easier reading.",
};

const PALETTE_COLORS: ReadonlyArray<{
  slot: PaletteColor;
  label: string;
  description: string;
}> = [
  { slot: "accent", label: "Accent", description: "Buttons and selections" },
  {
    slot: "background",
    label: "Background",
    description: "Main workspace and canvas",
  },
  { slot: "surface", label: "Surface", description: "Cards and controls" },
  { slot: "icon", label: "Icons", description: "Navigation and tool icons" },
];

export function AppearanceSettings({
  controls,
  description,
  children,
}: {
  controls: AppearanceControls;
  description: string;
  children?: import("react").ReactNode;
}) {
  const {
    colorTheme,
    darkPalette,
    setDarkPalette,
    resolvedColorTheme,
    setColorTheme,
    setTextSize,
    textSize,
    showSecurityWarnings,
    setShowSecurityWarnings,
    palettes,
    setPaletteColor,
    resetPalette,
  } = controls;
  const [editedTheme, setEditedTheme] =
    useState<PaletteTheme>(resolvedColorTheme);
  const [paletteError, setPaletteError] = useState<string | null>(null);
  const palette = palettes[editedTheme];
  const presetPreview = darkPalette !== "colossus" && editedTheme === "dark";
  const previewStyle = palettePreviewVariables(
    editedTheme,
    palette,
  ) as CSSProperties;

  return (
    <section
      className="managed-settings-body desktop-settings"
      aria-labelledby="appearance-settings-heading"
    >
      <div className="managed-section-heading">
        <div>
          <h3 id="appearance-settings-heading">Appearance</h3>
          <p className="managed-heading-copy">{description}</p>
        </div>
        <span className="status-chip tone-neutral" aria-live="polite">
          {resolvedColorTheme === "dark" ? "Dark palette" : "Light palette"}
        </span>
      </div>
      <div className="appearance-settings-card">
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
          <label htmlFor="appearance-dark-palette">
            <span className="appearance-control-icon">
              <IconPalette size={18} aria-hidden="true" />
            </span>
            <span className="appearance-control-copy">
              <strong>Dark palette</strong>
              <small id="appearance-dark-palette-help">
                Choose Colossus blue, neutral dark, or the TUI’s Hacker palette.
                Your custom Colossus colors are kept.
              </small>
            </span>
            <DropdownSelect
              id="appearance-dark-palette"
              value={darkPalette}
              aria-describedby="appearance-dark-palette-help"
              onChange={(event) =>
                setDarkPalette(event.target.value as DarkPalettePreference)
              }
            >
              <option value="colossus">Colossus blue</option>
              <option value="neutral">Neutral dark (Dark+)</option>
              <option value="hacker">Hacker (TUI)</option>
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
        {presetPreview ? (
          <p className="appearance-palette-notice">
            {darkPalette === "hacker" ? "Hacker" : "Neutral dark"} uses preset
            colors. Choose Colossus blue to edit your saved custom palette.
          </p>
        ) : null}
        <div className="appearance-palette-editor">
          <div className="appearance-palette-heading">
            <div>
              <h4>
                {presetPreview ? "Saved Colossus colors" : "Theme colors"}
              </h4>
              <p>
                {presetPreview
                  ? "These are your saved custom colors. Choose Colossus blue to edit or apply them."
                  : "Edit light and dark colors separately. The active theme updates immediately; the other palette is ready when you switch."}
              </p>
            </div>
            <div
              className="appearance-palette-tabs"
              role="group"
              aria-label="Palette to edit"
            >
              {(["light", "dark"] as const).map((theme) => (
                <button
                  key={theme}
                  type="button"
                  aria-pressed={editedTheme === theme}
                  onClick={() => {
                    setEditedTheme(theme);
                    setPaletteError(null);
                  }}
                >
                  {theme === "light" ? "Light colors" : "Dark colors"}
                </button>
              ))}
            </div>
          </div>
          <div className="appearance-palette-colors">
            {PALETTE_COLORS.map(({ slot, label, description }) => (
              <label key={slot} className="appearance-palette-color">
                <span>
                  <strong>{label}</strong>
                  <small>{description}</small>
                </span>
                <input
                  type="color"
                  disabled={presetPreview}
                  aria-label={`${editedTheme === "dark" ? "Dark" : "Light"} ${label.toLowerCase()} color`}
                  value={palette[slot]}
                  onInput={(event) => {
                    const accepted = setPaletteColor(
                      editedTheme,
                      slot,
                      event.currentTarget.value,
                    );
                    setPaletteError(
                      accepted
                        ? null
                        : slot === "icon"
                          ? "Choose an icon color that stands out on the background and surface."
                          : `Choose a ${editedTheme === "dark" ? "darker" : "lighter"} ${label.toLowerCase()} color so text and icons stay readable.`,
                    );
                  }}
                />
                <code>{palette[slot]}</code>
              </label>
            ))}
          </div>
          <div className="appearance-palette-footer">
            <p role="status" aria-live="polite">
              {paletteError ??
                "Background, surface, and icon colors keep content readable."}
            </p>
            <button
              type="button"
              className="button secondary compact"
              disabled={presetPreview || isDefaultPalette(editedTheme, palette)}
              onClick={() => {
                resetPalette(editedTheme);
                setPaletteError(null);
              }}
            >
              <IconRefresh size={16} aria-hidden="true" />
              Reset {editedTheme} colors
            </button>
          </div>
        </div>
        <div
          className="appearance-theme-preview"
          data-preview-theme={editedTheme}
          data-colossus-theme={editedTheme}
          data-palette={editedTheme === "dark" ? darkPalette : "colossus"}
          aria-label={`${editedTheme} theme preview`}
          style={presetPreview ? undefined : previewStyle}
        >
          <div className="appearance-preview-rail" aria-hidden="true">
            <span />
            <span className="appearance-preview-icon">
              <IconPalette size={14} stroke={1.8} />
            </span>
            <span />
          </div>
          <div className="appearance-preview-content">
            <strong>Theme preview</strong>
            <span>Workspace · Your next idea starts here</span>
            <i aria-hidden="true" />
          </div>
        </div>
        {setShowSecurityWarnings ? (
          <label className="compact-switch appearance-security-warnings">
            <input
              className="switch-input"
              type="checkbox"
              role="switch"
              checked={showSecurityWarnings}
              aria-labelledby="appearance-security-warnings-label"
              aria-describedby="appearance-security-warnings-help"
              onChange={(event) =>
                setShowSecurityWarnings(event.target.checked)
              }
            />
            <span>
              <strong id="appearance-security-warnings-label">
                Show security warnings
              </strong>
              <small id="appearance-security-warnings-help">
                Show Developer Preview and Full access banners at the top of the
                app. Off by default.
              </small>
            </span>
          </label>
        ) : null}
        {children}
      </div>
    </section>
  );
}
