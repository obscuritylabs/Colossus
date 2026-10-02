import { createContext, useContext, useEffect, useMemo, useState } from "react";
import type { PropsWithChildren } from "react";
import {
  DESKTOP_PREFERENCES_KEY,
  DEFAULT_DESKTOP_PREFERENCES,
  parseDesktopPreferences,
  readDesktopPreferences,
  storeDesktopPreferences,
} from "./desktop-preferences";
import type { DesktopPreferences } from "./desktop-preferences";

interface DesktopPreferencesContextValue extends DesktopPreferences {
  updatePreferences: (changes: Partial<DesktopPreferences>) => void;
}

const DesktopPreferencesContext = createContext<DesktopPreferencesContextValue>(
  {
    ...DEFAULT_DESKTOP_PREFERENCES,
    updatePreferences: () => {},
  },
);

export function DesktopPreferencesProvider({ children }: PropsWithChildren) {
  const [preferences, setPreferences] = useState(readDesktopPreferences);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === DESKTOP_PREFERENCES_KEY || event.key === null)
        setPreferences(parseDesktopPreferences(event.newValue));
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  const value = useMemo<DesktopPreferencesContextValue>(
    () => ({
      ...preferences,
      updatePreferences: (changes) =>
        setPreferences((current) => {
          const next = { ...current, ...changes };
          storeDesktopPreferences(next);
          return next;
        }),
    }),
    [preferences],
  );
  return (
    <DesktopPreferencesContext.Provider value={value}>
      {children}
    </DesktopPreferencesContext.Provider>
  );
}

export function useDesktopPreferences() {
  return useContext(DesktopPreferencesContext);
}
