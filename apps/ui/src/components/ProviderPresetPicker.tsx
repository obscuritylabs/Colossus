import type { ReactNode } from "react";
import { DropdownSelect } from "./DropdownSelect.js";

/** Hosts supply approved presets, icons and selection effects. */
export function ProviderPresetPicker({
  presets,
  selected,
  disabled = false,
  placeholder = "Choose a provider",
  onSelect,
  icon,
}: {
  presets: readonly { id: string; label: string }[];
  selected: string;
  disabled?: boolean;
  placeholder?: string;
  onSelect: (id: string) => void;
  icon?: (id: string) => ReactNode;
}) {
  return (
    <label className="provider-preset-control">
      <span>Provider</span>
      <DropdownSelect
        aria-label="Provider"
        value={selected}
        disabled={disabled}
        {...(icon
          ? {
              renderOptionIcon: (option: { value: string }) =>
                icon(option.value),
            }
          : {})}
        onChange={(event) => onSelect(event.target.value)}
      >
        <option value="">{placeholder}</option>
        {presets.map((preset) => (
          <option key={preset.id} value={preset.id}>
            {preset.label}
          </option>
        ))}
      </DropdownSelect>
    </label>
  );
}
