import { createContext, useEffect, useState } from "react";
import { DropdownSelect } from "@colossus/ui";

type Appearance = {
  colorTheme: "system" | "dark" | "light";
  darkPalette: "colossus" | "neutral" | "hacker";
  textSize: "compact" | "comfortable" | "large";
  sendShortcut: "enter" | "modEnter";
};
const storageKey = "colossus.web.appearance.v1";
const defaults: Appearance = {
  colorTheme: "system",
  darkPalette: "colossus",
  textSize: "comfortable",
  sendShortcut: "enter",
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
      sendShortcut: value.sendShortcut === "modEnter" ? "modEnter" : "enter",
      colorTheme: ["system", "dark", "light"].includes(value.colorTheme ?? "")
        ? value.colorTheme!
        : defaults.colorTheme,
      darkPalette: ["colossus", "neutral", "hacker"].includes(
        value.darkPalette ?? "",
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
}: {
  appearance: Appearance;
  onChange: (appearance: Appearance) => void;
}) {
  return (
    <section
      className="managed-settings-body appearance-settings"
      aria-labelledby="appearance-heading"
    >
      <header className="catalog-heading">
        <div>
          <h2 id="appearance-heading">Appearance</h2>
          <p>Display preferences for this browser.</p>
        </div>
      </header>
      <div className="appearance-fields">
        <label>
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
                sendShortcut: event.target.value as Appearance["sendShortcut"],
              })
            }
          >
            <option value="enter">Enter to send</option>
            <option value="modEnter">Ctrl / Cmd + Enter to send</option>
          </DropdownSelect>
        </label>
        <label>
          <span>
            <strong>Color theme</strong>
            <small>Follow your system or choose a light or dark theme.</small>
          </span>
          <DropdownSelect
            aria-label="Color theme"
            value={appearance.colorTheme}
            onChange={(event) =>
              onChange({
                ...appearance,
                colorTheme: event.target.value as Appearance["colorTheme"],
              })
            }
          >
            <option value="system">System</option>
            <option value="dark">Dark</option>
            <option value="light">Light</option>
          </DropdownSelect>
        </label>
        <label>
          <span>
            <strong>Dark palette</strong>
            <small>Used when the color theme is dark.</small>
          </span>
          <DropdownSelect
            aria-label="Dark palette"
            value={appearance.darkPalette}
            onChange={(event) =>
              onChange({
                ...appearance,
                darkPalette: event.target.value as Appearance["darkPalette"],
              })
            }
          >
            <option value="colossus">Colossus blue</option>
            <option value="neutral">Neutral dark</option>
            <option value="hacker">Hacker</option>
          </DropdownSelect>
        </label>
        <label>
          <span>
            <strong>Text size</strong>
            <small>
              Scale text across navigation, controls, and task output.
            </small>
          </span>
          <DropdownSelect
            aria-label="Text size"
            value={appearance.textSize}
            onChange={(event) =>
              onChange({
                ...appearance,
                textSize: event.target.value as Appearance["textSize"],
              })
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
