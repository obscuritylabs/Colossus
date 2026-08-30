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
  ResolvedColorTheme,
  TextSizePreference,
} from "./appearance";

interface AppearanceContextValue extends AppearancePreference {
  resolvedColorTheme: ResolvedColorTheme;
  setColorTheme: (theme: ColorThemePreference) => void;
  setTextSize: (size: TextSizePreference) => void;
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
      setTextSize: (textSize) =>
        setPreference((current) => ({ ...current, textSize })),
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
