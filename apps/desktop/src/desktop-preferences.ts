import { webLink } from "./components/browser/BrowserLink";

export const DESKTOP_PREFERENCES_KEY = "colossus.desktop.tools.v1";

export interface DesktopPreferences {
  gitAutoRefresh: boolean;
  gitDefaultView: "changes" | "history";
  browserNewTabUrl: string;
  terminalDefaultSession: "automatic" | "shell";
}

export const DEFAULT_DESKTOP_PREFERENCES: DesktopPreferences = {
  gitAutoRefresh: true,
  gitDefaultView: "changes",
  browserNewTabUrl: "",
  terminalDefaultSession: "automatic",
};

export function validBrowserNewTabUrl(value: string): string | null {
  if (value === "") return "";
  return webLink(value);
}

export function parseDesktopPreferences(
  raw: string | null,
): DesktopPreferences {
  if (raw === null) return DEFAULT_DESKTOP_PREFERENCES;
  try {
    const value = JSON.parse(raw) as Record<string, unknown>;
    return {
      gitAutoRefresh: value.gitAutoRefresh !== false,
      gitDefaultView:
        value.gitDefaultView === "history" ? "history" : "changes",
      browserNewTabUrl:
        typeof value.browserNewTabUrl === "string"
          ? (validBrowserNewTabUrl(value.browserNewTabUrl) ?? "")
          : "",
      terminalDefaultSession:
        value.terminalDefaultSession === "shell" ? "shell" : "automatic",
    };
  } catch {
    return DEFAULT_DESKTOP_PREFERENCES;
  }
}

export function readDesktopPreferences(): DesktopPreferences {
  try {
    return parseDesktopPreferences(
      localStorage.getItem(DESKTOP_PREFERENCES_KEY),
    );
  } catch {
    return DEFAULT_DESKTOP_PREFERENCES;
  }
}

export function storeDesktopPreferences(preferences: DesktopPreferences): void {
  try {
    localStorage.setItem(DESKTOP_PREFERENCES_KEY, JSON.stringify(preferences));
  } catch {
    // Device preferences remain usable for this session if storage is unavailable.
  }
}
