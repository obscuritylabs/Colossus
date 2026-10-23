import { createContext, useEffect, useState } from "react";
import { DropdownSelect } from "@colossus/ui";
import {
  AppearanceSettings as SharedAppearanceSettings,
  ClassificationColorsEditor,
} from "@colossus/ui/appearance";
import {
  parseClassificationColors,
  type ClassificationColors,
} from "@colossus/ui/lib/classification-colors";
import { ApiFailure, request } from "./api";
import type { AdminSettings, PublicSettings } from "./control-api";
import { errorMessage, useResource } from "./resources";
import { LoadState } from "./Home";
import {
  DARK_PALETTE_OPTIONS,
  DEFAULT_THEME_PALETTES,
  PALETTE_CSS_VARIABLES,
  parseThemePalettes,
  paletteCssVariables,
  validPaletteColor,
  type DarkPalettePreference,
  type ThemePalettes,
} from "@colossus/ui/lib/palette";
import "@colossus/ui/styles/appearance.css";

type Appearance = {
  colorTheme: "system" | "dark" | "light";
  darkPalette: DarkPalettePreference;
  textSize: "compact" | "comfortable" | "large";
  sendShortcut: "enter" | "modEnter";
  palettes: ThemePalettes;
};
const storageKey = "colossus.web.appearance.v1";
const defaults: Appearance = {
  colorTheme: "system",
  darkPalette: "colossus",
  textSize: "comfortable",
  sendShortcut: "enter",
  palettes: parseThemePalettes(DEFAULT_THEME_PALETTES),
};
export const SendShortcutContext = createContext<"enter" | "modEnter">("enter");

function readAppearance(): Appearance {
  try {
    const stored = localStorage.getItem(storageKey);
    if (!stored) {
      const legacy = localStorage.getItem("colossus-cloud-theme");
      return {
        ...defaults,
        colorTheme: legacy === "light" || legacy === "dark" ? legacy : "system",
      };
    }
    const value = JSON.parse(stored) as Partial<Appearance>;
    return {
      palettes: parseThemePalettes(value.palettes),
      sendShortcut: value.sendShortcut === "modEnter" ? "modEnter" : "enter",
      colorTheme: ["system", "dark", "light"].includes(value.colorTheme ?? "")
        ? value.colorTheme!
        : defaults.colorTheme,
      darkPalette: DARK_PALETTE_OPTIONS.includes(
        value.darkPalette ?? defaults.darkPalette,
      )
        ? value.darkPalette!
        : defaults.darkPalette,
      textSize: ["compact", "comfortable", "large"].includes(
        value.textSize ?? "",
      )
        ? value.textSize!
        : defaults.textSize,
    };
  } catch {
    return defaults;
  }
}

/** Browser preferences stay in the web host, outside the shared presentation. */
export function useAppearance() {
  const [appearance, setAppearance] = useState(readAppearance);
  useEffect(() => {
    const system = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const root = document.documentElement;
      root.dataset.theme =
        appearance.colorTheme === "system"
          ? system.matches
            ? "dark"
            : "light"
          : appearance.colorTheme;
      root.dataset.palette = appearance.darkPalette;
      root.dataset.textSize = appearance.textSize;
      root.dataset.themePreference = appearance.colorTheme;
      for (const property of PALETTE_CSS_VARIABLES)
        root.style.removeProperty(property);
      const resolved = root.dataset.theme === "light" ? "light" : "dark";
      if (resolved === "light" || appearance.darkPalette === "colossus")
        for (const [name, value] of Object.entries(
          paletteCssVariables(resolved, appearance.palettes[resolved]),
        ))
          root.style.setProperty(name, value);
    };
    apply();
    system.addEventListener("change", apply);
    try {
      localStorage.setItem(storageKey, JSON.stringify(appearance));
    } catch {
      /* Preferences are best-effort. */
    }
    return () => system.removeEventListener("change", apply);
  }, [appearance]);
  return { appearance, setAppearance };
}

export function AppearanceSettings({
  appearance,
  onChange,
  isGlobalAdmin = false,
  onEnvironmentSettings,
}: {
  appearance: Appearance;
  onChange: (appearance: Appearance) => void;
  isGlobalAdmin?: boolean;
  onEnvironmentSettings?: (settings: PublicSettings) => void;
}) {
  const resolvedColorTheme =
    appearance.colorTheme === "system"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light"
      : appearance.colorTheme;
  return (
    <SharedAppearanceSettings
      description={
        isGlobalAdmin
          ? "Theme and text preferences stay local to this browser. You can also publish classification banner colors for this Control Plane below."
          : "Choose how Colossus looks in this browser. Changes apply immediately and stay local to this browser."
      }
      controls={{
        ...appearance,
        resolvedColorTheme,
        setColorTheme: (value) =>
          onChange({ ...appearance, colorTheme: value }),
        setDarkPalette: (value) =>
          onChange({ ...appearance, darkPalette: value }),
        setTextSize: (value) => onChange({ ...appearance, textSize: value }),
        setPaletteColor: (theme, slot, value) => {
          if (
            !validPaletteColor(theme, slot, value, appearance.palettes[theme])
          )
            return false;
          onChange({
            ...appearance,
            palettes: {
              ...appearance.palettes,
              [theme]: {
                ...appearance.palettes[theme],
                [slot]: value.toLowerCase(),
              },
            },
          });
          return true;
        },
        resetPalette: (theme) =>
          onChange({
            ...appearance,
            palettes: {
              ...appearance.palettes,
              [theme]: { ...DEFAULT_THEME_PALETTES[theme] },
            },
          }),
      }}
    >
      <label className="appearance-send-shortcut">
        <span>
          <strong>Send shortcut</strong>
          <small>Shift+Enter inserts a new line.</small>
        </span>
        <DropdownSelect
          aria-label="Send shortcut"
          value={appearance.sendShortcut}
          onChange={(event) =>
            onChange({
              ...appearance,
              sendShortcut:
                event.target.value === "modEnter" ? "modEnter" : "enter",
            })
          }
        >
          <option value="enter">Enter to send</option>
          <option value="modEnter">Ctrl / Cmd + Enter to send</option>
        </DropdownSelect>
      </label>
      {isGlobalAdmin && onEnvironmentSettings ? (
        <EnvironmentAppearance onSaved={onEnvironmentSettings} />
      ) : null}
    </SharedAppearanceSettings>
  );
}

/** Administrator-published deployment colors remain separate from browser preferences. */
export function EnvironmentAppearance({
  onSaved,
}: {
  onSaved: (settings: PublicSettings) => void;
}) {
  const resource = useResource<AdminSettings>("/api/admin/settings");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [saved, setSaved] = useState<AdminSettings | null>(null);
  const settings =
    saved && saved.revision >= (resource.data?.revision ?? 0)
      ? saved
      : resource.data;
  async function saveColors(colors: ClassificationColors | null) {
    if (!settings || busy) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const updated = await request<PublicSettings & { revision: number }>(
        "/api/admin/settings",
        {
          revision: settings.revision,
          classification: { ...settings.classification, colors },
        },
        undefined,
        "PATCH",
      );
      setSaved({ ...settings, ...updated });
      onSaved(updated);
      setNotice(
        "Classification banner colors published for this Control Plane.",
      );
      resource.refresh();
    } catch (cause) {
      setError(
        cause instanceof ApiFailure &&
          [400, 401, 403, 409].includes(cause.status)
          ? errorMessage(cause)
          : "Could not confirm whether banner colors were published. Current settings are reloading; review them before publishing again.",
      );
      resource.refresh();
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="appearance-environment-settings">
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <p className="alert" role="alert">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p className="notice" role="status">
          {notice}
        </p>
      ) : null}
      {settings ? (
        <ClassificationColorsEditor
          colors={parseClassificationColors(settings.classification.colors)}
          onChange={(colors) => void saveColors(colors)}
          previewText={settings.classification.text || "Classification banner"}
          description={`Publish background and text colors for everyone in this Control Plane, including sign-in. Only global administrators can edit these colors.${settings.classification.enabled ? "" : " The classification banner is currently hidden."}`}
          disabled={busy || resource.loading}
          applyLabel="Publish banner colors"
        />
      ) : null}
    </div>
  );
}
