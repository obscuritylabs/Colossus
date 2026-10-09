import { useId, useRef, type ReactNode } from "react";
import { IconCircle, IconCircleCheck } from "@tabler/icons-react";
import { Button } from "./Controls.js";

export interface RadioGroupOption {
  value: string;
  label: string;
  description?: ReactNode;
}

/** Accessible choices composed from the shared shadcn button foundation. */
export function RadioGroup({
  value,
  onValueChange,
  options,
  disabled = false,
  variant = "cards",
  "aria-label": label,
}: {
  value: string;
  onValueChange: (value: string) => void;
  options: RadioGroupOption[];
  disabled?: boolean;
  variant?: "cards" | "compact";
  "aria-label": string;
}) {
  const id = useId();
  const controls = useRef<(HTMLButtonElement | null)[]>([]);
  const selected = options.findIndex((option) => option.value === value);
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={
        variant === "compact"
          ? "ui-radio-group ui-radio-group--compact"
          : "ui-radio-group ui:flex ui:flex-col ui:gap-2"
      }
    >
      {options.map((option, index) => {
        const checked = value === option.value;
        return (
          <Button
            key={option.value}
            role="radio"
            aria-checked={checked}
            aria-describedby={option.description ? `${id}-${index}` : undefined}
            ref={(element) => {
              controls.current[index] = element;
            }}
            disabled={disabled}
            tabIndex={index === Math.max(0, selected) ? 0 : -1}
            variant="secondary"
            className={
              variant === "compact"
                ? "ui-radio-option--compact"
                : `ui:h-auto ui:justify-start ui:items-start ui:gap-3 ui:px-3 ui:py-3 ui:text-left ui:whitespace-normal ${checked ? "ui:border-primary ui:bg-accent" : ""}`
            }
            onClick={() => onValueChange(option.value)}
            onKeyDown={(event) => {
              const step =
                event.key === "ArrowDown" || event.key === "ArrowRight"
                  ? 1
                  : event.key === "ArrowUp" || event.key === "ArrowLeft"
                    ? -1
                    : 0;
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? options.length - 1
                    : step
                      ? (index + step + options.length) % options.length
                      : -1;
              if (next < 0) return;
              event.preventDefault();
              onValueChange(options[next]!.value);
              controls.current[next]?.focus();
            }}
          >
            {checked ? (
              <IconCircleCheck
                className="ui:shrink-0"
                size={18}
                aria-hidden="true"
              />
            ) : (
              <IconCircle
                className="ui:shrink-0"
                size={18}
                aria-hidden="true"
              />
            )}
            <span className="ui:min-w-0 ui:flex ui:flex-col ui:gap-1">
              <span>{option.label}</span>
              {option.description && (
                <span
                  id={`${id}-${index}`}
                  className="ui:font-normal ui:text-xs ui:text-muted-foreground"
                >
                  {option.description}
                </span>
              )}
            </span>
          </Button>
        );
      })}
    </div>
  );
}
