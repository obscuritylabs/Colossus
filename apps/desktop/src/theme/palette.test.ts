import { describe, expect, it } from "vitest";

import {
  DEFAULT_THEME_PALETTES,
  paletteCssVariables,
  parseThemePalettes,
  validPaletteColor,
} from "./palette";

describe("editable theme palettes", () => {
  it("accepts separate light and dark colors while rejecting unreadable or malformed values", () => {
    expect(validPaletteColor("dark", "background", "#16243a")).toBe(true);
    expect(validPaletteColor("light", "surface", "#edf1f8")).toBe(true);
    expect(validPaletteColor("dark", "background", "#ffffff")).toBe(false);
    expect(validPaletteColor("light", "surface", "#000000")).toBe(false);
    expect(validPaletteColor("light", "accent", "red; color: white")).toBe(
      false,
    );
    expect(validPaletteColor("dark", "icon", "#50d9a8")).toBe(true);
    expect(validPaletteColor("light", "icon", "#a1366f")).toBe(true);
    expect(validPaletteColor("light", "icon", "#ffffff")).toBe(false);

    expect(
      parseThemePalettes({
        dark: {
          accent: "#C04B61",
          background: "#ffffff",
          icon: "#50D9A8",
        },
        light: {
          accent: "#087E8B",
          surface: "#000000",
          icon: "#ffffff",
        },
      }),
    ).toEqual({
      dark: {
        ...DEFAULT_THEME_PALETTES.dark,
        accent: "#c04b61",
        icon: "#50d9a8",
      },
      light: {
        ...DEFAULT_THEME_PALETTES.light,
        accent: "#087e8b",
      },
    });
  });

  it("derives the app's semantic colors from each palette", () => {
    const colors = paletteCssVariables("dark", {
      accent: "#c04b61",
      background: "#101e30",
      surface: "#1a2c42",
      icon: "#50d9a8",
    });
    expect(colors["--blue"]).toBe("#c04b61");
    expect(colors["--main"]).toBe("#101e30");
    expect(colors["--surface"]).toBe("#1a2c42");
    expect(colors["--icon-text"]).toBe("#50d9a8");
    expect(colors["--navigation-icon"]).toBe("#50d9a8");
    expect(colors["--blue-soft"]).not.toBe(colors["--surface"]);
  });
});
