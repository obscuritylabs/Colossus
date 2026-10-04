export interface Preferences {
  sendShortcut: "enter" | "modEnter";
  defaultMode: "plan" | "execute";
  showToolActivity: boolean;
  palette: "editor" | "colossus" | "hacker";
}

export const DEFAULT_PREFERENCES: Preferences = {
  sendShortcut: "modEnter",
  defaultMode: "plan",
  showToolActivity: true,
  palette: "editor",
};

export const PREFERENCE_KEYS = {
  sendShortcut: "composer.sendShortcut",
  defaultMode: "composer.defaultMode",
  showToolActivity: "appearance.showToolActivity",
  palette: "appearance.palette",
} as const;

export interface SettingsView {
  preferences: Preferences;
  workspace: string;
  connected: boolean;
  connecting: boolean;
  busy: boolean;
  hasSavedConnection: boolean;
  version: string;
  role: string;
  error: string;
}

export type SettingsAction =
  | {
      type:
        | "ready"
        | "connect"
        | "disconnect"
        | "forgetConnection"
        | "configureCredential"
        | "showConnectionLog"
        | "openWork"
        | "openThemeSettings";
    }
  | { type: "setPreference"; name: "sendShortcut"; value: "enter" | "modEnter" }
  | { type: "setPreference"; name: "defaultMode"; value: "plan" | "execute" }
  | { type: "setPreference"; name: "showToolActivity"; value: boolean }
  | { type: "setPreference"; name: "palette"; value: Preferences["palette"] };

export function parseSettingsAction(
  value: unknown,
): SettingsAction | undefined {
  if (!value || typeof value !== "object") return;
  const input = value as Record<string, unknown>;
  if (
    Object.keys(input).length === 1 &&
    typeof input.type === "string" &&
    [
      "ready",
      "connect",
      "disconnect",
      "forgetConnection",
      "configureCredential",
      "showConnectionLog",
      "openWork",
      "openThemeSettings",
    ].includes(input.type)
  )
    return input as SettingsAction;
  if (input.type !== "setPreference" || Object.keys(input).length !== 3) return;
  if (
    (input.name === "sendShortcut" &&
      (input.value === "enter" || input.value === "modEnter")) ||
    (input.name === "defaultMode" &&
      (input.value === "plan" || input.value === "execute")) ||
    (input.name === "showToolActivity" && typeof input.value === "boolean") ||
    (input.name === "palette" &&
      (input.value === "editor" ||
        input.value === "colossus" ||
        input.value === "hacker"))
  )
    return input as SettingsAction;
  return;
}

// Invalid externally edited settings fall back to the same defaults as the manifest.
export function readPreferences(get: (key: string) => unknown): Preferences {
  const shortcut = get(PREFERENCE_KEYS.sendShortcut);
  const mode = get(PREFERENCE_KEYS.defaultMode);
  const activity = get(PREFERENCE_KEYS.showToolActivity);
  const palette = get(PREFERENCE_KEYS.palette);
  return {
    sendShortcut: shortcut === "enter" ? "enter" : "modEnter",
    defaultMode: mode === "execute" ? "execute" : "plan",
    showToolActivity: typeof activity === "boolean" ? activity : true,
    palette:
      palette === "colossus" || palette === "hacker" ? palette : "editor",
  };
}
