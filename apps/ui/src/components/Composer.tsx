import {
  useRef,
  type RefObject,
  type TextareaHTMLAttributes,
  type ButtonHTMLAttributes,
} from "react";
import { IconSend2 } from "@tabler/icons-react";
import { useComposerAutosize } from "../hooks/useComposerAutosize.js";

/** Presentation only: the host decides when a draft is accepted or cleared. */
export function ComposerInput({
  ref,
  value,
  className = "",
  ...props
}: Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "value"> & {
  value: string;
  ref?: RefObject<HTMLTextAreaElement | null>;
}) {
  const localRef = useRef<HTMLTextAreaElement>(null);
  const textareaRef = ref ?? localRef;
  useComposerAutosize(textareaRef, value);
  return (
    <textarea
      {...props}
      ref={textareaRef}
      value={value}
      rows={props.rows ?? 2}
      maxLength={props.maxLength ?? 65_536}
      className={`composer-input ${className}`.trim()}
    />
  );
}

export function ComposerModeSwitch<T extends string>({
  value,
  onChange,
  options,
  disabled = false,
  name = "mode",
}: {
  value: T;
  onChange: (value: T) => void;
  options: readonly {
    value: T;
    label: string;
    disabled?: boolean;
    title?: string;
  }[];
  disabled?: boolean;
  name?: string;
}) {
  return (
    <fieldset className="mode-switch">
      <legend className="sr-only">Run mode</legend>
      {options.map((option) => (
        <label key={option.value}>
          <input
            id={`mode-${option.value}`}
            type="radio"
            name={name}
            value={option.value}
            checked={value === option.value}
            disabled={disabled || option.disabled}
            onChange={() => onChange(option.value)}
          />
          <span title={option.title}>{option.label}</span>
        </label>
      ))}
    </fieldset>
  );
}

export function ComposerSendButton({
  children,
  className = "",
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...props}
      type={type}
      className={`send-button ${className}`.trim()}
    >
      {children ?? <IconSend2 size={19} stroke={2} aria-hidden="true" />}
    </button>
  );
}

export function isComposerSendKey(
  event: {
    key: string;
    shiftKey: boolean;
    altKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
    isComposing: boolean;
  },
  shortcut: "enter" | "modEnter",
) {
  return (
    !event.isComposing &&
    event.key === "Enter" &&
    !event.shiftKey &&
    !event.altKey &&
    (shortcut === "enter" || event.ctrlKey || event.metaKey)
  );
}
