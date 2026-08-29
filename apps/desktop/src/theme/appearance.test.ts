import { describe, expect, it } from "vitest";

import {
  APPEARANCE_STORAGE_KEY,
  DEFAULT_APPEARANCE,
  applyAppearance,
  parseAppearancePreference,
  readAppearancePreference,
  resolveColorTheme,
  storeAppearancePreference,
} from "./appearance";

describe("appearance preferences", () => {
  it("falls back safely for missing, invalid, and partially invalid values", () => {
    expect(parseAppearancePreference(null)).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearancePreference("not json")).toEqual(DEFAULT_APPEARANCE);
    expect(
      parseAppearancePreference(
        JSON.stringify({ colorTheme: "light", textSize: "enormous" }),
      ),
    ).toEqual({ colorTheme: "light", textSize: "comfortable" });
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
    });

    expect(values.get(APPEARANCE_STORAGE_KEY)).toBe(
      '{"colorTheme":"dark","textSize":"large"}',
    );
    expect(readAppearancePreference(storage)).toEqual({
      colorTheme: "dark",
      textSize: "large",
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
      applyAppearance(root, { colorTheme: "system", textSize: "large" }, false),
    ).toBe("light");
    expect(Object.fromEntries(attributes)).toEqual({
      "data-theme": "light",
      "data-theme-preference": "system",
      "data-text-size": "large",
    });
  });
});
