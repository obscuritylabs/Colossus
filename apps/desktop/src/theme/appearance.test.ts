import { describe, expect, it } from "vitest";

import {
  APPEARANCE_STORAGE_KEY,
  DEFAULT_APPEARANCE,
  applyAppearance,
  appearanceStorage,
  parseAppearancePreference,
  readAppearancePreference,
  readHostAppearancePreference,
  resolveColorTheme,
  storeAppearancePreference,
  storeHostAppearancePreference,
  subscribeToAppearancePreference,
} from "./appearance";

describe("appearance preferences", () => {
  it("falls back safely for missing, invalid, and partially invalid values", () => {
    expect(parseAppearancePreference(null)).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearancePreference("not json")).toEqual(DEFAULT_APPEARANCE);
    expect(
      parseAppearancePreference(
        JSON.stringify({ colorTheme: "light", textSize: "enormous" }),
      ),
    ).toEqual({ ...DEFAULT_APPEARANCE, colorTheme: "light" });
  });

  it("keeps security warnings off for new and existing preferences", () => {
    expect(DEFAULT_APPEARANCE.showSecurityWarnings).toBe(false);
    expect(
      parseAppearancePreference(
        JSON.stringify({ colorTheme: "dark", textSize: "large" }),
      ),
    ).toEqual({
      colorTheme: "dark",
      textSize: "large",
      showSecurityWarnings: false,
    });
    for (const value of [false, null, "true", 1]) {
      expect(
        parseAppearancePreference(
          JSON.stringify({ showSecurityWarnings: value }),
        ).showSecurityWarnings,
      ).toBe(false);
    }
  });

  it("reads and writes the versioned device-local preference", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    storeAppearancePreference(storage, {
      colorTheme: "dark",
      textSize: "large",
      showSecurityWarnings: true,
    });

    expect(values.get(APPEARANCE_STORAGE_KEY)).toBe(
      '{"colorTheme":"dark","textSize":"large","showSecurityWarnings":true}',
    );
    expect(readAppearancePreference(storage)).toEqual({
      colorTheme: "dark",
      textSize: "large",
      showSecurityWarnings: true,
    });
  });

  it("does not let unavailable storage block startup", () => {
    const unavailable = {
      getItem: () => {
        throw new Error("unavailable");
      },
      setItem: () => {
        throw new Error("unavailable");
      },
    };

    expect(readAppearancePreference(unavailable)).toEqual(DEFAULT_APPEARANCE);
    expect(() =>
      storeAppearancePreference(unavailable, DEFAULT_APPEARANCE),
    ).not.toThrow();

    const deniedHost = {
      get localStorage(): never {
        throw new Error("denied");
      },
    };
    expect(appearanceStorage(deniedHost)).toBeNull();
    expect(readHostAppearancePreference(deniedHost)).toEqual(
      DEFAULT_APPEARANCE,
    );
    expect(() =>
      storeHostAppearancePreference(deniedHost, DEFAULT_APPEARANCE),
    ).not.toThrow();
  });

  it("synchronizes only local appearance storage changes", () => {
    const localStorage = {
      getItem: () => null,
      setItem: () => undefined,
    };
    const sessionStorage = {
      getItem: () => null,
      setItem: () => undefined,
    };
    let storageListener:
      | ((event: {
          key: string | null;
          newValue: string | null;
          storageArea: typeof localStorage | null;
        }) => void)
      | undefined;
    const target = {
      addEventListener: (
        _type: "storage",
        listener: typeof storageListener,
      ) => {
        storageListener = listener;
      },
      removeEventListener: (
        _type: "storage",
        listener: typeof storageListener,
      ) => {
        if (storageListener === listener) {
          storageListener = undefined;
        }
      },
    };
    const observed: (typeof DEFAULT_APPEARANCE)[] = [];
    const unsubscribe = subscribeToAppearancePreference(
      target,
      localStorage,
      (preference) => observed.push(preference),
    );

    storageListener?.({
      key: "unrelated",
      newValue: JSON.stringify({ colorTheme: "dark", textSize: "large" }),
      storageArea: localStorage,
    });
    storageListener?.({
      key: APPEARANCE_STORAGE_KEY,
      newValue: JSON.stringify({ colorTheme: "dark", textSize: "large" }),
      storageArea: sessionStorage,
    });
    storageListener?.({
      key: APPEARANCE_STORAGE_KEY,
      newValue: JSON.stringify({ colorTheme: "dark", textSize: "large" }),
      storageArea: localStorage,
    });
    storageListener?.({ key: null, newValue: null, storageArea: localStorage });

    expect(observed).toEqual([
      { ...DEFAULT_APPEARANCE, colorTheme: "dark", textSize: "large" },
      DEFAULT_APPEARANCE,
    ]);
    unsubscribe();
    expect(storageListener).toBeUndefined();
  });

  it("resolves system color and applies all root state attributes", () => {
    const attributes = new Map<string, string>();
    const root = {
      setAttribute: (name: string, value: string) =>
        attributes.set(name, value),
    };

    expect(resolveColorTheme("system", true)).toBe("dark");
    expect(resolveColorTheme("system", false)).toBe("light");
    expect(resolveColorTheme("dark", false)).toBe("dark");

    expect(
      applyAppearance(
        root,
        { ...DEFAULT_APPEARANCE, textSize: "large" },
        false,
      ),
    ).toBe("light");
    expect(Object.fromEntries(attributes)).toEqual({
      "data-theme": "light",
      "data-theme-preference": "system",
      "data-text-size": "large",
    });
  });
});
