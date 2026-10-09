import { describe, expect, it } from "vitest";

import {
  APPEARANCE_STORAGE_KEY,
  DEFAULT_APPEARANCE,
  applyAppearance,
  appearanceStorage,
  parseAppearancePreference,
  readAppearancePreference,
  readHostAppearancePreference,
  readNativeDialogAppearance,
  resolveColorTheme,
  storeAppearancePreference,
  storeHostAppearancePreference,
  subscribeToAppearancePreference,
} from "./appearance";
import { DEFAULT_THEME_PALETTES } from "./palette";

describe("appearance preferences", () => {
  it("captures only the rendered native-dialog theme and text size", () => {
    const values = new Map([
      ["data-theme", "dark"],
      ["data-text-size", "large"],
      ["data-unrelated", "not-sent"],
    ]);
    const root = { getAttribute: (name: string) => values.get(name) ?? null };
    expect(readNativeDialogAppearance(root)).toEqual({
      colorScheme: "dark",
      textSize: "large",
    });
    values.set("data-theme", "light");
    values.set("data-text-size", "compact");
    expect(readNativeDialogAppearance(root)).toEqual({
      colorScheme: "light",
      textSize: "compact",
    });
    values.set("data-theme", "arbitrary-css");
    values.delete("data-text-size");
    expect(readNativeDialogAppearance(root)).toEqual({
      colorScheme: "system",
      textSize: "comfortable",
    });
  });

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
      darkPalette: "colossus",
      textSize: "large",
      showSecurityWarnings: false,
      palettes: DEFAULT_THEME_PALETTES,
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
      darkPalette: "colossus",
      textSize: "large",
      showSecurityWarnings: true,
      palettes: DEFAULT_THEME_PALETTES,
    });

    expect(JSON.parse(values.get(APPEARANCE_STORAGE_KEY)!)).toEqual({
      colorTheme: "dark",
      darkPalette: "colossus",
      textSize: "large",
      showSecurityWarnings: true,
      palettes: DEFAULT_THEME_PALETTES,
    });
    expect(readAppearancePreference(storage)).toEqual({
      colorTheme: "dark",
      darkPalette: "colossus",
      textSize: "large",
      showSecurityWarnings: true,
      palettes: DEFAULT_THEME_PALETTES,
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
    const properties = new Map<string, string>();
    const root = {
      setAttribute: (name: string, value: string) =>
        attributes.set(name, value),
      style: {
        setProperty: (name: string, value: string) =>
          properties.set(name, value),
        removeProperty: (name: string) => {
          const previous = properties.get(name) ?? "";
          properties.delete(name);
          return previous;
        },
      },
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
      "data-palette": "colossus",
      "data-theme-preference": "system",
      "data-text-size": "large",
    });
    expect(properties.size).toBe(0);

    applyAppearance(
      root,
      {
        ...DEFAULT_APPEARANCE,
        palettes: {
          ...DEFAULT_THEME_PALETTES,
          light: { ...DEFAULT_THEME_PALETTES.light, accent: "#c04b61" },
        },
      },
      false,
    );
    expect(properties.get("--blue")).toBe("#c04b61");
    applyAppearance(root, DEFAULT_APPEARANCE, false);
    expect(properties.size).toBe(0);
  });
});

it.each(["neutral", "black", "hacker"] as const)(
  "persists %s without changing legacy defaults or saved custom colors",
  (darkPalette) => {
    expect(
      parseAppearancePreference(JSON.stringify({ darkPalette: "invalid" }))
        .darkPalette,
    ).toBe("colossus");
    const preference = {
      ...DEFAULT_APPEARANCE,
      colorTheme: "dark" as const,
      darkPalette,
      palettes: {
        ...DEFAULT_THEME_PALETTES,
        dark: { ...DEFAULT_THEME_PALETTES.dark, background: "#101010" },
      },
    };
    expect(parseAppearancePreference(JSON.stringify(preference))).toEqual(
      preference,
    );
    const attributes = new Map<string, string>();
    const properties = new Map<string, string>();
    const root = {
      setAttribute: (name: string, value: string) => {
        attributes.set(name, value);
      },
      style: {
        setProperty: (name: string, value: string) => {
          properties.set(name, value);
        },
        removeProperty: (name: string) => {
          properties.delete(name);
          return "";
        },
      },
    };
    applyAppearance(root, preference, false);
    expect(attributes.get("data-palette")).toBe(darkPalette);
    expect(properties.has("--main")).toBe(false);
    applyAppearance(
      root,
      {
        ...preference,
        colorTheme: "light",
        palettes: {
          ...preference.palettes,
          light: { ...DEFAULT_THEME_PALETTES.light, background: "#fafafa" },
        },
      },
      false,
    );
    expect(properties.get("--main")).toBe("#fafafa");
    applyAppearance(root, { ...preference, colorTheme: "system" }, true);
    expect(attributes.get("data-theme")).toBe("dark");
    expect(properties.has("--main")).toBe(false);
    applyAppearance(root, { ...preference, darkPalette: "colossus" }, false);
    expect(properties.get("--main")).toBe("#101010");
  },
);
