export const DARK_PALETTE_OPTIONS = [
  "colossus",
  "neutral",
  "black",
  "hacker",
] as const;
export type DarkPalettePreference = (typeof DARK_PALETTE_OPTIONS)[number];

export type PaletteTheme = "dark" | "light";
export type PaletteColor = "accent" | "background" | "surface" | "icon";

export interface ThemePalette {
  accent: string;
  background: string;
  surface: string;
  icon: string;
}

export type ThemePalettes = Record<PaletteTheme, ThemePalette>;

export const DEFAULT_THEME_PALETTES: ThemePalettes = {
  dark: {
    accent: "#4389ff",
    background: "#0d1928",
    surface: "#111f30",
    icon: "#76adff",
  },
  light: {
    accent: "#2563d9",
    background: "#f7f9fc",
    surface: "#ffffff",
    icon: "#245fae",
  },
};

const THEME_TEXT = {
  dark: { strong: "#f7faff", muted: "#91a2b8" },
  light: { strong: "#102033", muted: "#52657a" },
} as const;

const HEX_COLOR = /^#[0-9a-f]{6}$/i;

export function normalizedPaletteColor(value: unknown): string | null {
  return typeof value === "string" && HEX_COLOR.test(value)
    ? value.toLowerCase()
    : null;
}

function channels(color: string): [number, number, number] {
  return [1, 3, 5].map((index) =>
    Number.parseInt(color.slice(index, index + 2), 16),
  ) as [number, number, number];
}

function luminance(color: string): number {
  const linear = (channel: number) => {
    const fraction = channel / 255;
    return fraction <= 0.04045
      ? fraction / 12.92
      : ((fraction + 0.055) / 1.055) ** 2.4;
  };
  const [red, green, blue] = channels(color);
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

function contrastRatio(first: string, second: string): number {
  const brighter = Math.max(luminance(first), luminance(second));
  const darker = Math.min(luminance(first), luminance(second));
  return (brighter + 0.05) / (darker + 0.05);
}

/** Keep the existing strong and muted text readable over editable surfaces. */
export function validPaletteColor(
  theme: PaletteTheme,
  slot: PaletteColor,
  value: unknown,
  palette: ThemePalette = DEFAULT_THEME_PALETTES[theme],
): value is string {
  const color = normalizedPaletteColor(value);
  if (color === null) return false;
  if (slot === "accent") return true;
  if (slot === "icon") {
    return (
      contrastRatio(color, palette.background) >= 3 &&
      contrastRatio(color, palette.surface) >= 3
    );
  }
  const text = THEME_TEXT[theme];
  return (
    contrastRatio(color, text.strong) >= 4.5 &&
    contrastRatio(color, text.muted) >= 4.5 &&
    contrastRatio(color, palette.icon) >= 3
  );
}

export function parseThemePalettes(value: unknown): ThemePalettes {
  const saved =
    value !== null && typeof value === "object"
      ? (value as Record<string, unknown>)
      : {};
  const palette = (theme: PaletteTheme): ThemePalette => {
    const savedTheme = saved[theme];
    const colors =
      savedTheme !== null && typeof savedTheme === "object"
        ? (savedTheme as Record<string, unknown>)
        : {};
    const fallback = DEFAULT_THEME_PALETTES[theme];
    const parse = (slot: PaletteColor, current: ThemePalette = fallback) =>
      validPaletteColor(theme, slot, colors[slot], current)
        ? normalizedPaletteColor(colors[slot])!
        : fallback[slot];
    const background = parse("background");
    const surface = parse("surface");
    return {
      accent: parse("accent"),
      background,
      surface,
      icon: parse("icon", { ...fallback, background, surface }),
    };
  };
  return { dark: palette("dark"), light: palette("light") };
}

function mix(first: string, second: string, secondWeight: number): string {
  const start = channels(first);
  const end = channels(second);
  return `#${start
    .map((channel, index) =>
      Math.round(channel * (1 - secondWeight) + end[index]! * secondWeight)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function readableAccent(theme: PaletteTheme, palette: ThemePalette): string {
  const destination = theme === "dark" ? "#ffffff" : "#000000";
  for (let step = 0; step <= 20; step += 1) {
    const candidate = mix(palette.accent, destination, step / 20);
    if (
      contrastRatio(candidate, palette.background) >= 4.5 &&
      contrastRatio(candidate, palette.surface) >= 4.5
    ) {
      return candidate;
    }
  }
  return destination;
}

export const PALETTE_CSS_VARIABLES = [
  "--canvas",
  "--rail",
  "--sidebar",
  "--main",
  "--surface",
  "--surface-raised",
  "--surface-muted",
  "--surface-hover",
  "--surface-inset",
  "--surface-panel",
  "--surface-header",
  "--surface-control",
  "--surface-editor",
  "--surface-selected",
  "--surface-selected-hover",
  "--border",
  "--border-strong",
  "--blue",
  "--blue-strong",
  "--blue-hover",
  "--blue-soft",
  "--text-on-accent",
  "--accent-text",
  "--accent-border",
  "--focus-ring",
  "--scroll-thumb",
  "--icon-surface",
  "--icon-border",
  "--icon-text",
  "--navigation-icon",
  "--code-surface",
  "--code-header",
] as const;

export function paletteCssVariables(
  theme: PaletteTheme,
  palette: ThemePalette,
): Record<(typeof PALETTE_CSS_VARIABLES)[number], string> {
  const { accent, background, surface, icon } = palette;
  const dark = theme === "dark";
  const neutral = dark ? "#000000" : "#102033";
  const text = THEME_TEXT[theme].strong;
  const strongAccent = readableAccent(theme, palette);
  const onAccent =
    contrastRatio(accent, "#ffffff") >= contrastRatio(accent, "#07101d")
      ? "#ffffff"
      : "#07101d";
  const rgb = channels(accent);
  return {
    "--canvas": mix(background, neutral, dark ? 0.16 : 0.04),
    "--rail": mix(background, neutral, dark ? 0.25 : 0.09),
    "--sidebar": mix(background, neutral, dark ? 0.12 : 0.05),
    "--main": background,
    "--surface": surface,
    "--surface-raised": mix(surface, dark ? "#ffffff" : background, 0.08),
    "--surface-muted": mix(background, surface, 0.25),
    "--surface-hover": mix(surface, accent, 0.1),
    "--surface-inset": mix(background, surface, 0.35),
    "--surface-panel": mix(background, surface, 0.5),
    "--surface-header": mix(surface, accent, 0.06),
    "--surface-control": dark ? mix(background, neutral, 0.05) : surface,
    "--surface-editor": mix(background, surface, 0.5),
    "--surface-selected": mix(surface, accent, dark ? 0.2 : 0.13),
    "--surface-selected-hover": mix(surface, accent, dark ? 0.27 : 0.2),
    "--border": mix(surface, text, dark ? 0.14 : 0.16),
    "--border-strong": mix(surface, text, dark ? 0.23 : 0.28),
    "--blue": accent,
    "--blue-strong": strongAccent,
    "--blue-hover": mix(accent, dark ? "#ffffff" : "#000000", 0.12),
    "--blue-soft": mix(surface, accent, dark ? 0.2 : 0.13),
    "--text-on-accent": onAccent,
    "--accent-text": strongAccent,
    "--accent-border": mix(surface, accent, dark ? 0.52 : 0.46),
    "--focus-ring": `rgb(${rgb.join(" ")} / 22%)`,
    "--scroll-thumb": mix(surface, text, dark ? 0.27 : 0.34),
    "--icon-surface": mix(surface, accent, dark ? 0.18 : 0.12),
    "--icon-border": mix(surface, accent, dark ? 0.34 : 0.3),
    "--icon-text": icon,
    "--navigation-icon":
      icon === DEFAULT_THEME_PALETTES[theme].icon ? "currentColor" : icon,
    "--code-surface": mix(background, neutral, dark ? 0.08 : 0.02),
    "--code-header": mix(background, neutral, dark ? 0.03 : 0.05),
  };
}

export function isDefaultPalette(
  theme: PaletteTheme,
  palette: ThemePalette,
): boolean {
  const defaultPalette = DEFAULT_THEME_PALETTES[theme];
  return (
    palette.accent === defaultPalette.accent &&
    palette.background === defaultPalette.background &&
    palette.surface === defaultPalette.surface &&
    palette.icon === defaultPalette.icon
  );
}

/** Preview uses the same readable text foundations as the active palette. */
export function palettePreviewVariables(
  theme: PaletteTheme,
  palette: ThemePalette,
): Record<string, string> {
  return {
    ...paletteCssVariables(theme, palette),
    "--text": THEME_TEXT[theme].strong,
    "--muted": THEME_TEXT[theme].muted,
  };
}
