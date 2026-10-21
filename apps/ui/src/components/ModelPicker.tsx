import { useId, useRef, useState, type ReactNode } from "react";
import { IconCheck, IconChevronDown, IconSearch } from "@tabler/icons-react";
import { Button } from "./ui/button.js";
import { Input } from "./ui/input.js";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover.js";
import { Slider } from "./ui/slider.js";

export interface ModelPickerOption {
  value: string;
  label: string;
  detail: string;
  icon?: ReactNode;
}

/** Presentation only: the host supplies configured models and reasoning choices. */
export function ModelPicker({
  model,
  models,
  onModelChange,
  label,
  detail,
  icon,
  reasoning,
  reasoningLabel,
  reasoningOptions,
  onReasoningChange,
  onOpenSettings,
  disabled = false,
  portalContainer,
}: {
  model: string;
  models: ModelPickerOption[];
  onModelChange: (value: string) => void;
  label: string;
  detail: string;
  icon?: ReactNode;
  reasoning: string;
  reasoningLabel: string;
  reasoningOptions: { value: string; label: string }[];
  onReasoningChange: (value: string) => void;
  onOpenSettings?: (() => void) | undefined;
  disabled?: boolean;
  portalContainer?: HTMLElement | null | undefined;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(model);
  const listId = useId();
  const slider = useRef<HTMLSpanElement>(null);
  const options = useRef(new Map<string, HTMLButtonElement>());
  const filtered = models.filter((option) =>
    `${option.label} ${option.detail}`
      .toLocaleLowerCase()
      .includes(query.toLocaleLowerCase()),
  );
  const activeIndex = Math.max(
    0,
    filtered.findIndex((option) => option.value === active),
  );
  const activeOption = filtered[activeIndex];
  const effortIndex = Math.max(
    0,
    reasoningOptions.findIndex((option) => option.value === reasoning),
  );
  function choose(value: string) {
    if (disabled) return;
    onModelChange(value);
    setActive(value);
    setQuery("");
    // Keep the popover open so reasoning is the next keyboard interaction.
    slider.current?.focus();
  }
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) {
          setQuery("");
          setActive(model);
        }
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          className="ui-model-trigger"
          disabled={disabled}
          aria-label={`Choose model and reasoning: ${label}`}
          title={`${label} · ${detail} · ${reasoningLabel}`}
        >
          {icon}
          <strong>{label}</strong>
          {reasoningOptions.length > 0 && (
            <span className="ui-model-trigger-effort">{reasoningLabel}</span>
          )}
          <IconChevronDown size={14} aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        portalContainer={portalContainer}
        onEscapeKeyDown={() => setOpen(false)}
        className="ui-model-picker"
        aria-label="Model and reasoning"
        side="top"
        collisionPadding={12}
      >
        <div className="ui-model-search">
          <IconSearch size={16} aria-hidden="true" />
          <Input
            role="combobox"
            disabled={disabled}
            aria-label="Search models"
            aria-autocomplete="list"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={
              activeOption
                ? `${listId}-${models.indexOf(activeOption)}`
                : undefined
            }
            placeholder="Search models or providers…"
            value={query}
            maxLength={128}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                if (activeOption) choose(activeOption.value);
              } else if (
                ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) &&
                filtered.length
              ) {
                event.preventDefault();
                const next =
                  event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? filtered.length - 1
                      : (activeIndex +
                          (event.key === "ArrowDown" ? 1 : -1) +
                          filtered.length) %
                        filtered.length;
                const value = filtered[next]!.value;
                setActive(value);
                options.current
                  .get(value)
                  ?.scrollIntoView({ block: "nearest" });
              }
            }}
          />
        </div>
        <div
          id={listId}
          role="listbox"
          aria-label="Models"
          className="ui-model-list"
        >
          {filtered.map((option) => (
            <button
              key={option.value}
              ref={(element) => {
                if (element) options.current.set(option.value, element);
                else options.current.delete(option.value);
              }}
              type="button"
              disabled={disabled}
              role="option"
              id={`${listId}-${models.indexOf(option)}`}
              aria-selected={model === option.value}
              data-active={activeOption === option}
              tabIndex={-1}
              className="ui-model-option"
              onClick={() => choose(option.value)}
            >
              {option.icon}
              <span className="ui-model-option-copy">
                <strong>{option.label}</strong>
                <span>{option.detail}</span>
              </span>
              {model === option.value && (
                <IconCheck size={16} aria-hidden="true" />
              )}
            </button>
          ))}
        </div>
        {!filtered.length && (
          <p role="status" className="ui-model-empty">
            No matching models
          </p>
        )}
        {reasoningOptions.length > 0 && (
          <div className="ui-model-reasoning">
            <div className="ui-model-reasoning-heading">
              <span>Reasoning</span>
              <strong aria-live="polite">{reasoningLabel}</strong>
            </div>
            <Slider
              ref={slider}
              disabled={disabled}
              min={0}
              max={reasoningOptions.length - 1}
              step={1}
              value={[effortIndex]}
              aria-label="Reasoning effort for next message"
              aria-valuetext={reasoningLabel}
              onValueChange={(values) => {
                const option = reasoningOptions[values[0]!];
                if (option) onReasoningChange(option.value);
              }}
            />
            <div className="ui-model-effort-scale" aria-hidden="true">
              <span>Default</span>
              <span>{reasoningOptions.at(-1)?.label}</span>
            </div>
          </div>
        )}
        <div className="ui-model-picker-footer">
          {onOpenSettings && (
            <Button
              variant="ghost"
              onClick={() => {
                setOpen(false);
                onOpenSettings();
              }}
            >
              Configure models
            </Button>
          )}
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Done
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
