import { TextInput } from "./Controls";

export const DEFAULT_GOAL_ITERATIONS = 5;
export const validGoalIterations = (value: number) =>
  Number.isInteger(value) && value >= 1 && value <= 50;

/** Presentation only; the host captures this explicit budget in its durable request. */
export function GoalControls({
  value,
  disabled = false,
  onChange,
}: {
  value: number;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <label>
      <span>Goal iterations</span>
      <TextInput
        type="number"
        aria-label="Goal iterations"
        min={1}
        max={50}
        step={1}
        value={Number.isFinite(value) ? value : ""}
        disabled={disabled}
        aria-invalid={!validGoalIterations(value)}
        onInput={(event) => onChange(event.currentTarget.valueAsNumber)}
      />
      <small className="field-help">
        Continue automatically for up to this many iterations. Each iteration
        uses the turn limit; normal permissions and approvals apply.
      </small>
    </label>
  );
}
