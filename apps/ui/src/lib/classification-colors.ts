import { contrastRatio, normalizedPaletteColor } from "./palette";
import type { CSSProperties } from "react";

export interface ClassificationColors {
  background: string;
  text: string;
}

/** Starting colors for the editor; no saved override keeps the host's tone tokens. */
export const DEFAULT_CLASSIFICATION_COLORS: ClassificationColors = {
  background: "#1e293b",
  text: "#ffffff",
};

/** Accept color data only, never arbitrary CSS, and keep ordinary banner text readable. */
export function parseClassificationColors(
  value: unknown,
): ClassificationColors | null {
  if (value === null || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  const background = normalizedPaletteColor(record.background);
  const text = normalizedPaletteColor(record.text);
  if (!background || !text || contrastRatio(background, text) < 4.5)
    return null;
  return { background, text };
}

/** Only validated literal colors may reach a host's banner CSS properties. */
export function classificationColorVariables(value: unknown):
  | (CSSProperties & {
      "--classification-background": string;
      "--classification-text": string;
    })
  | undefined {
  const colors = parseClassificationColors(value);
  return colors
    ? {
        "--classification-background": colors.background,
        "--classification-text": colors.text,
      }
    : undefined;
}
