import { IconRefresh } from "@tabler/icons-react";
import { useEffect, useState } from "react";

import {
  DEFAULT_CLASSIFICATION_COLORS,
  classificationColorVariables,
  parseClassificationColors,
  type ClassificationColors,
} from "../lib/classification-colors";
import { contrastRatio } from "../lib/palette";
import { Button } from "./Controls";

export interface ClassificationColorsControls {
  colors: ClassificationColors | null;
  onChange: (colors: ClassificationColors | null) => void;
  previewText?: string;
  description?: string;
  disabled?: boolean;
  applyLabel?: string;
}

/** A host-controlled color override; publishing, persistence and marking policy stay in the host. */
export function ClassificationColorsEditor({
  colors,
  onChange,
  previewText = "Classification banner",
  description = "Choose the banner background and text colors. The classification label and placement stay the same.",
  disabled = false,
  applyLabel = "Apply banner colors",
}: ClassificationColorsControls) {
  const background = parseClassificationColors(colors)?.background;
  const text = parseClassificationColors(colors)?.text;
  const [draft, setDraft] = useState<ClassificationColors>(
    () =>
      parseClassificationColors(colors) ?? { ...DEFAULT_CLASSIFICATION_COLORS },
  );
  useEffect(() => {
    setDraft(
      background && text
        ? { background, text }
        : { ...DEFAULT_CLASSIFICATION_COLORS },
    );
  }, [background, text]);
  const accepted = parseClassificationColors(draft);
  const changed = draft.background !== background || draft.text !== text;
  const previewStyle = classificationColorVariables(accepted);

  return (
    <section
      className="appearance-palette-editor appearance-classification-editor"
      aria-label="Classification banner colors"
    >
      <div className="appearance-palette-heading">
        <div>
          <h4>Classification banner colors</h4>
          <p>{description}</p>
          {!colors ? (
            <p>
              Tone colors are active. The preview shows your proposed custom
              colors.
            </p>
          ) : null}
        </div>
      </div>
      <div className="appearance-palette-colors">
        {(["background", "text"] as const).map((slot) => (
          <label key={slot} className="appearance-palette-color">
            <span>
              <strong>{slot === "background" ? "Background" : "Text"}</strong>
              <small>
                {slot === "background"
                  ? "Banner background"
                  : "Classification label"}
              </small>
            </span>
            <input
              type="color"
              aria-label={`Classification ${slot} color`}
              value={draft[slot]}
              disabled={disabled}
              onInput={(event) => {
                const value = event.currentTarget.value;
                setDraft((current) => ({
                  ...current,
                  [slot]: value,
                }));
              }}
            />
            <code>{draft[slot]}</code>
          </label>
        ))}
      </div>
      <div
        className="appearance-classification-preview"
        aria-label="Classification color preview"
        style={previewStyle}
      >
        {previewText}
      </div>
      <div className="appearance-palette-footer">
        <p role="status" aria-live="polite">
          {accepted
            ? `Text contrast ${contrastRatio(accepted.background, accepted.text).toFixed(1)}:1. Colors require at least 4.5:1 contrast.`
            : "Choose background and text colors with at least 4.5:1 contrast before applying them."}
        </p>
        <Button
          variant="secondary"
          type="button"
          disabled={disabled || !colors}
          onClick={() => onChange(null)}
        >
          <IconRefresh size={16} aria-hidden="true" />
          Use tone colors
        </Button>
        <Button
          variant="primary"
          type="button"
          disabled={disabled || !accepted || !changed}
          onClick={() => {
            if (accepted) onChange(accepted);
          }}
        >
          {applyLabel}
        </Button>
      </div>
    </section>
  );
}
