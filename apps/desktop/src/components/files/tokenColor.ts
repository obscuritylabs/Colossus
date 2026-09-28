import type { ResolvedColorTheme } from "../../theme/appearance";

// GitHub Light colors target a white canvas. Darken them slightly for the
// Desktop's tinted source/diff backgrounds, preserving syntax distinctions.
export function sourceTokenColor(
  color: string | undefined,
  theme: ResolvedColorTheme,
) {
  return theme === "light" && color
    ? `color-mix(in srgb, ${color} 80%, var(--text))`
    : color;
}
