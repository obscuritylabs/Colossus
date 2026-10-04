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
  preferenceError: string;
  workspace: string;
  connected: boolean;
  connecting: boolean;
  busy: boolean;
  reconnectable?: boolean;
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
        | "openThemeSettings"
        | "openUserSettings";
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
      "openUserSettings",
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

// Native configuration errors can contain user paths or setting contents. Only
// translate recognized failure categories into fixed, public instructions.
export function preferenceSaveError(
  name: Extract<SettingsAction, { type: "setPreference" }>["name"],
  error: unknown,
): string {
  const labels = {
    palette: "Surface palette",
    sendShortcut: "Send shortcut",
    defaultMode: "Default run mode",
    showToolActivity: "Show tool activity",
  };
  const message =
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
      ? error.message.toLowerCase()
      : "";
  const reason = message.includes("not a registered configuration")
    ? "VS Code has not registered this setting. Reload Window after installing Colossus, then try again."
    : message.includes("configured in system policy")
      ? "This setting is managed by system policy."
      : message.includes("file has unsaved changes")
        ? "The VS Code user settings file has unsaved changes. Save it, then try again."
        : message.includes("content of the file is newer")
          ? "The VS Code user settings file changed on disk. Reload that file, then try again."
          : message.includes(
                "open the user settings to correct errors/warnings",
              )
            ? "The VS Code user settings file contains errors. Correct them, then try again."
            : /eacces|eperm|permission denied|read.only|nopermissions/u.test(
                  message,
                )
              ? "VS Code cannot write its user settings file. Check that the file is writable, then try again."
              : "VS Code could not update its user settings. Check the user settings file for errors and write access, then try again.";
  return `Could not save ${labels[name]}. ${reason}`;
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
