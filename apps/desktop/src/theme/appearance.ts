export const COLOR_THEME_OPTIONS = ["system", "dark", "light"] as const;
export const TEXT_SIZE_OPTIONS = ["compact", "comfortable", "large"] as const;

export type ColorThemePreference = (typeof COLOR_THEME_OPTIONS)[number];
export type ResolvedColorTheme = Exclude<ColorThemePreference, "system">;
export type TextSizePreference = (typeof TEXT_SIZE_OPTIONS)[number];

export interface AppearancePreference {
  colorTheme: ColorThemePreference;
  textSize: TextSizePreference;
}

export interface AppearanceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface AppearanceRoot {
  setAttribute(name: string, value: string): void;
}

export const APPEARANCE_STORAGE_KEY = "colossus.desktop.appearance.v1";

export const DEFAULT_APPEARANCE: AppearancePreference = {
  colorTheme: "system",
  textSize: "comfortable",
};

function includes<const T extends readonly string[]>(
  options: T,
  value: unknown,
): value is T[number] {
  return typeof value === "string" && options.includes(value);
}

export function parseAppearancePreference(
  serialized: string | null,
): AppearancePreference {
  if (serialized === null) {
    return DEFAULT_APPEARANCE;
  }
  try {
    const value = JSON.parse(serialized) as Record<string, unknown>;
    return {
      colorTheme: includes(COLOR_THEME_OPTIONS, value.colorTheme)
        ? value.colorTheme
        : DEFAULT_APPEARANCE.colorTheme,
      textSize: includes(TEXT_SIZE_OPTIONS, value.textSize)
        ? value.textSize
        : DEFAULT_APPEARANCE.textSize,
    };
  } catch {
    return DEFAULT_APPEARANCE;
  }
}

export function readAppearancePreference(
  storage: AppearanceStorage,
): AppearancePreference {
  try {
    return parseAppearancePreference(storage.getItem(APPEARANCE_STORAGE_KEY));
  } catch {
    return DEFAULT_APPEARANCE;
  }
}

export function storeAppearancePreference(
  storage: AppearanceStorage,
  preference: AppearancePreference,
) {
  try {
    storage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(preference));
  } catch {
    // Appearance preferences are best-effort and must never block the app.
  }
}

export function resolveColorTheme(
  preference: ColorThemePreference,
  systemPrefersDark: boolean,
): ResolvedColorTheme {
  if (preference === "system") {
    return systemPrefersDark ? "dark" : "light";
  }
  return preference;
}

export function applyAppearance(
  root: AppearanceRoot,
  preference: AppearancePreference,
  systemPrefersDark: boolean,
) {
  const resolved = resolveColorTheme(preference.colorTheme, systemPrefersDark);
  root.setAttribute("data-theme", resolved);
  root.setAttribute("data-theme-preference", preference.colorTheme);
  root.setAttribute("data-text-size", preference.textSize);
  return resolved;
}
