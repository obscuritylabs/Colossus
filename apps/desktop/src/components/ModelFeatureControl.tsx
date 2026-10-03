import type { ModelFeatureMode } from "../types";
import "./model-feature-control.css";

const MODES = ["off", "auto", "on"] as const;

export function ModelFeatureControl({
  label,
  description,
  declared,
  value,
  disabled,
  onChange,
}: {
  label: string;
  description?: string;
  declared?: boolean | null | undefined;
  value: ModelFeatureMode;
  disabled?: boolean;
  onChange: (mode: ModelFeatureMode) => void;
}) {
  return (
    <fieldset className="model-feature-control" disabled={disabled}>
      <legend>{label}</legend>
      {description ? <small>{description}</small> : null}
      <small>
        {declared === true
          ? "Model card reports support."
          : declared === false
            ? "Model card reports unsupported."
            : "Model-card support is unknown."}
      </small>
      <input
        type="range"
        min={0}
        max={2}
        step={1}
        value={MODES.indexOf(value)}
        aria-label={label}
        aria-valuetext={
          value === "auto" ? "Auto" : value === "on" ? "On" : "Off"
        }
        onChange={(event) =>
          onChange(MODES[Number(event.target.value)] ?? "auto")
        }
      />
      <div className="model-feature-positions">
        {MODES.map((mode) => (
          <button
            key={mode}
            type="button"
            aria-pressed={value === mode}
            onClick={() => onChange(mode)}
          >
            {mode === "auto" ? "Auto" : mode === "on" ? "On" : "Off"}
          </button>
        ))}
      </div>
    </fieldset>
  );
}
