import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
} from "react";
import type { PropsWithChildren } from "react";

import {
  DEFAULT_APPEARANCE,
  applyAppearance,
  appearanceStorage,
  readHostAppearancePreference,
  resolveColorTheme,
  storeHostAppearancePreference,
  subscribeToAppearancePreference,
} from "./appearance";
import type {
  AppearancePreference,
  ColorThemePreference,
  DarkPalettePreference,
  ResolvedColorTheme,
  TextSizePreference,
} from "./appearance";
import {
  DEFAULT_THEME_PALETTES,
  normalizedPaletteColor,
  validPaletteColor,
} from "./palette";
import type { PaletteColor, PaletteTheme } from "./palette";

interface AppearanceContextValue extends AppearancePreference {
  resolvedColorTheme: ResolvedColorTheme;
  setColorTheme: (theme: ColorThemePreference) => void;
  setDarkPalette: (palette: DarkPalettePreference) => void;
  setTextSize: (size: TextSizePreference) => void;
  setShowSecurityWarnings: (show: boolean) => void;
  setPaletteColor: (
    theme: PaletteTheme,
    slot: PaletteColor,
    color: string,
  ) => boolean;
  resetPalette: (theme: PaletteTheme) => void;
}

const AppearanceContext = createContext<AppearanceContextValue | null>(null);

const SYSTEM_DARK_QUERY = "(prefers-color-scheme: dark)";

function readSystemPreference() {
  return (
    typeof window !== "undefined" &&
    window.matchMedia(SYSTEM_DARK_QUERY).matches
  );
}

function readInitialPreference() {
  if (typeof window === "undefined") {
    return DEFAULT_APPEARANCE;
  }
  return readHostAppearancePreference(window);
}

export function initializeAppearance() {
  const preference = readInitialPreference();
  if (typeof document !== "undefined") {
    applyAppearance(
      document.documentElement,
      preference,
      readSystemPreference(),
    );
  }
  return preference;
}

export function AppearanceProvider({
  children,
  initialPreference,
}: PropsWithChildren<{ initialPreference?: AppearancePreference }>) {
  const [preference, setPreference] = useState(
    () => initialPreference ?? readInitialPreference(),
  );
  const [systemPrefersDark, setSystemPrefersDark] =
    useState(readSystemPreference);

  useEffect(() => {
    const media = window.matchMedia(SYSTEM_DARK_QUERY);
    const handleChange = (event: MediaQueryListEvent) => {
      setSystemPrefersDark(event.matches);
    };
    setSystemPrefersDark(media.matches);
    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, []);

  useEffect(() => {
    const storage = appearanceStorage(window);
    if (storage === null) {
      return;
    }
    return subscribeToAppearancePreference(window, storage, setPreference);
  }, []);

  useLayoutEffect(() => {
    applyAppearance(document.documentElement, preference, systemPrefersDark);
    storeHostAppearancePreference(window, preference);
  }, [preference, systemPrefersDark]);

  const value = useMemo<AppearanceContextValue>(
    () => ({
      ...preference,
      resolvedColorTheme: resolveColorTheme(
        preference.colorTheme,
        systemPrefersDark,
      ),
      setColorTheme: (colorTheme) =>
        setPreference((current) => ({ ...current, colorTheme })),
      setDarkPalette: (darkPalette) =>
        setPreference((current) => ({ ...current, darkPalette })),
      setTextSize: (textSize) =>
        setPreference((current) => ({ ...current, textSize })),
      setShowSecurityWarnings: (showSecurityWarnings) =>
        setPreference((current) => ({ ...current, showSecurityWarnings })),
      setPaletteColor: (theme, slot, value) => {
        if (
          !validPaletteColor(theme, slot, value, preference.palettes[theme])
        ) {
          return false;
        }
        const color = normalizedPaletteColor(value)!;
        setPreference((current) => ({
          ...current,
          palettes: {
            ...current.palettes,
            [theme]: { ...current.palettes[theme], [slot]: color },
          },
        }));
        return true;
      },
      resetPalette: (theme) =>
        setPreference((current) => ({
          ...current,
          palettes: {
            ...current.palettes,
            [theme]: DEFAULT_THEME_PALETTES[theme],
          },
        })),
    }),
    [preference, systemPrefersDark],
  );

  return (
    <AppearanceContext.Provider value={value}>
      {children}
    </AppearanceContext.Provider>
  );
}

export function useAppearance() {
  const value = useContext(AppearanceContext);
  if (value === null) {
    throw new Error("useAppearance must be used inside AppearanceProvider");
  }
  return value;
}
