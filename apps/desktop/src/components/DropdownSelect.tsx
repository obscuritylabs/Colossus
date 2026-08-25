import { IconCheck, IconChevronDown } from "@tabler/icons-react";
import {
  Children,
  Fragment,
  isValidElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  OptionHTMLAttributes,
  ReactElement,
  ReactNode,
} from "react";
import { createPortal } from "react-dom";

export interface DropdownSelectChangeEvent {
  target: { value: string };
  currentTarget: { value: string };
}

export interface DropdownSelectOption {
  value: string;
  label: string;
  disabled: boolean;
}

interface DropdownPosition {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  placement: "above" | "below";
}

interface DropdownSelectProps {
  children?: ReactNode;
  value?: string | number;
  defaultValue?: string | number;
  disabled?: boolean;
  required?: boolean;
  className?: string;
  id?: string;
  title?: string;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  "aria-describedby"?: string;
  onChange?: (event: DropdownSelectChangeEvent) => void;
}

function optionText(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }
  if (Array.isArray(node)) {
    return node.map(optionText).join("");
  }
  if (isValidElement(node)) {
    return optionText((node.props as { children?: ReactNode }).children);
  }
  return "";
}

export function dropdownOptions(children: ReactNode): DropdownSelectOption[] {
  const options: DropdownSelectOption[] = [];

  function visit(nodes: ReactNode) {
    Children.forEach(nodes, (child) => {
      if (!isValidElement(child)) {
        return;
      }
      if (child.type === Fragment) {
        visit((child.props as { children?: ReactNode }).children);
        return;
      }
      if (child.type !== "option") {
        return;
      }
      const option = child as ReactElement<
        OptionHTMLAttributes<HTMLOptionElement>
      >;
      options.push({
        value: String(option.props.value ?? ""),
        label: optionText(option.props.children).trim(),
        disabled: option.props.disabled === true,
      });
    });
  }

  visit(children);
  return options;
}

export function nextDropdownOptionIndex(
  options: readonly DropdownSelectOption[],
  current: number,
  direction: 1 | -1,
): number {
  if (options.length === 0) {
    return -1;
  }
  for (let step = 1; step <= options.length; step += 1) {
    const candidate =
      (current + direction * step + options.length) % options.length;
    if (!options[candidate]?.disabled) {
      return candidate;
    }
  }
  return -1;
}

function edgeOptionIndex(
  options: readonly DropdownSelectOption[],
  edge: "first" | "last",
): number {
  if (edge === "first") {
    return options.findIndex((option) => !option.disabled);
  }
  for (let index = options.length - 1; index >= 0; index -= 1) {
    if (!options[index]?.disabled) {
      return index;
    }
  }
  return -1;
}

export function DropdownSelect({
  children,
  value,
  defaultValue,
  disabled = false,
  required = false,
  className = "",
  id,
  title,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
  "aria-describedby": ariaDescribedBy,
  onChange,
}: DropdownSelectProps) {
  const options = useMemo(() => dropdownOptions(children), [children]);
  const generatedId = useId();
  const listboxId = `${id ?? generatedId}-listbox`;
  const controlled = value !== undefined;
  const [localValue, setLocalValue] = useState(() =>
    String(defaultValue ?? options[0]?.value ?? ""),
  );
  const selectedValue = controlled ? String(value) : localValue;
  const selectedIndex = options.findIndex(
    (option) => option.value === selectedValue,
  );
  const selectedOption =
    selectedIndex >= 0 ? options[selectedIndex] : undefined;
  const selectedLabel =
    selectedOption?.label ??
    (selectedValue !== ""
      ? `Unavailable: ${selectedValue}`
      : options.length === 0
        ? "No options"
        : "Select an option");
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(selectedIndex);
  const [position, setPosition] = useState<DropdownPosition | null>(null);
  const rootRef = useRef<HTMLSpanElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listboxRef = useRef<HTMLDivElement>(null);
  const typeaheadRef = useRef("");
  const typeaheadTimerRef = useRef<number | null>(null);

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (trigger === null) {
      return;
    }
    const rect = trigger.getBoundingClientRect();
    const viewportPadding = 8;
    const gap = 5;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const width = Math.min(
      Math.max(rect.width, 180),
      viewportWidth - viewportPadding * 2,
    );
    const measuredHeight = Math.min(
      listboxRef.current?.scrollHeight ?? 240,
      280,
    );
    const below = viewportHeight - rect.bottom - gap - viewportPadding;
    const above = rect.top - gap - viewportPadding;
    const placement =
      below < Math.min(measuredHeight, 160) && above > below
        ? "above"
        : "below";
    const available = Math.max(72, placement === "above" ? above : below);
    const menuHeight = Math.min(measuredHeight, available);
    const left = Math.min(
      Math.max(viewportPadding, rect.left),
      Math.max(viewportPadding, viewportWidth - viewportPadding - width),
    );
    setPosition({
      top:
        placement === "above"
          ? Math.max(viewportPadding, rect.top - gap - menuHeight)
          : rect.bottom + gap,
      left,
      width,
      maxHeight: available,
      placement,
    });
  }, []);

  const openMenu = useCallback(
    (initialIndex = selectedIndex) => {
      if (disabled || options.length === 0) {
        return;
      }
      const fallback = edgeOptionIndex(options, "first");
      const initial = initialIndex >= 0 ? options[initialIndex] : undefined;
      setActiveIndex(
        initial !== undefined && !initial.disabled ? initialIndex : fallback,
      );
      setOpen(true);
    },
    [disabled, options, selectedIndex],
  );

  const closeMenu = useCallback(() => {
    setOpen(false);
    setPosition(null);
  }, []);

  const chooseOption = useCallback(
    (index: number) => {
      const option = options[index];
      if (option === undefined || option.disabled) {
        return;
      }
      if (!controlled) {
        setLocalValue(option.value);
      }
      const change = {
        target: { value: option.value },
        currentTarget: { value: option.value },
      };
      onChange?.(change);
      closeMenu();
      window.requestAnimationFrame(() => triggerRef.current?.focus());
    },
    [closeMenu, controlled, onChange, options],
  );

  const useTypeahead = useCallback(
    (key: string) => {
      if (typeaheadTimerRef.current !== null) {
        window.clearTimeout(typeaheadTimerRef.current);
      }
      typeaheadRef.current = `${typeaheadRef.current}${key.toLocaleLowerCase()}`;
      typeaheadTimerRef.current = window.setTimeout(() => {
        typeaheadRef.current = "";
        typeaheadTimerRef.current = null;
      }, 600);
      const start = Math.max(activeIndex, selectedIndex);
      for (let step = 1; step <= options.length; step += 1) {
        const candidate = (start + step) % options.length;
        const option = options[candidate];
        if (
          option !== undefined &&
          !option.disabled &&
          option.label.toLocaleLowerCase().startsWith(typeaheadRef.current)
        ) {
          setActiveIndex(candidate);
          if (!open) {
            openMenu(candidate);
          }
          return;
        }
      }
    },
    [activeIndex, open, openMenu, options, selectedIndex],
  );

  function handleKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>) {
    if (disabled) {
      return;
    }
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp": {
        event.preventDefault();
        const direction = event.key === "ArrowDown" ? 1 : -1;
        const current = open ? activeIndex : selectedIndex;
        const next =
          current >= 0
            ? nextDropdownOptionIndex(options, current, direction)
            : edgeOptionIndex(options, direction === 1 ? "first" : "last");
        if (!open) {
          openMenu(next);
        } else if (next >= 0) {
          setActiveIndex(next);
        }
        return;
      }
      case "Home":
      case "End": {
        event.preventDefault();
        const next = edgeOptionIndex(
          options,
          event.key === "Home" ? "first" : "last",
        );
        if (!open) {
          openMenu(next);
        } else {
          setActiveIndex(next);
        }
        return;
      }
      case "Enter":
      case " ":
        event.preventDefault();
        if (open) {
          chooseOption(activeIndex);
        } else {
          openMenu();
        }
        return;
      case "Escape":
        if (open) {
          event.preventDefault();
          closeMenu();
        }
        return;
      case "Tab":
        closeMenu();
        return;
      default:
        if (event.key.length === 1 && !event.metaKey && !event.ctrlKey) {
          useTypeahead(event.key);
        }
    }
  }

  useLayoutEffect(() => {
    if (!open) {
      return undefined;
    }
    updatePosition();
    window.addEventListener("resize", updatePosition);
    document.addEventListener("scroll", updatePosition, true);
    return () => {
      window.removeEventListener("resize", updatePosition);
      document.removeEventListener("scroll", updatePosition, true);
    };
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) {
      return;
    }
    listboxRef.current
      ?.querySelector<HTMLElement>(`[data-option-index="${activeIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, open]);

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    function closeOnOutsidePointer(event: PointerEvent) {
      const target = event.target;
      if (
        target instanceof Node &&
        !rootRef.current?.contains(target) &&
        !listboxRef.current?.contains(target)
      ) {
        closeMenu();
      }
    }
    document.addEventListener("pointerdown", closeOnOutsidePointer);
    return () =>
      document.removeEventListener("pointerdown", closeOnOutsidePointer);
  }, [closeMenu, open]);

  useEffect(() => {
    if (disabled) {
      closeMenu();
    }
  }, [closeMenu, disabled]);

  useEffect(
    () => () => {
      if (typeaheadTimerRef.current !== null) {
        window.clearTimeout(typeaheadTimerRef.current);
      }
    },
    [],
  );

  const optionId = (index: number) => `${listboxId}-option-${index}`;
  const menuStyle: CSSProperties =
    position === null
      ? { visibility: "hidden" }
      : {
          top: position.top,
          left: position.left,
          width: position.width,
          maxHeight: position.maxHeight,
        };

  return (
    <span
      className={`app-select${className === "" ? "" : ` ${className}`}`}
      data-disabled={disabled ? "true" : "false"}
      data-open={open ? "true" : "false"}
      ref={rootRef}
    >
      <button
        className="app-select-trigger"
        type="button"
        id={id}
        title={title}
        role="combobox"
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledBy}
        aria-describedby={ariaDescribedBy}
        aria-controls={listboxId}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-required={required || undefined}
        aria-invalid={required && selectedValue === "" ? true : undefined}
        aria-activedescendant={
          open && activeIndex >= 0 ? optionId(activeIndex) : undefined
        }
        disabled={disabled}
        ref={triggerRef}
        onClick={() => (open ? closeMenu() : openMenu())}
        onKeyDown={handleKeyDown}
      >
        <span className="app-select-value">{selectedLabel}</span>
        <IconChevronDown
          className="app-select-chevron"
          size={15}
          stroke={2}
          aria-hidden="true"
        />
      </button>
      {required ? (
        <input
          className="app-select-validity-proxy"
          type="text"
          value={selectedValue}
          required
          disabled={disabled}
          tabIndex={-1}
          aria-hidden="true"
          onChange={() => undefined}
          onInvalid={(event) => {
            event.preventDefault();
            triggerRef.current?.focus();
          }}
        />
      ) : null}
      {open && typeof document !== "undefined"
        ? createPortal(
            <div
              className="app-select-popover"
              id={listboxId}
              role="listbox"
              aria-label={ariaLabel}
              aria-labelledby={ariaLabelledBy}
              data-placement={position?.placement ?? "below"}
              ref={listboxRef}
              style={menuStyle}
            >
              {options.map((option, index) => (
                <button
                  className="app-select-option"
                  id={optionId(index)}
                  type="button"
                  role="option"
                  aria-selected={index === selectedIndex}
                  data-active={index === activeIndex ? "true" : "false"}
                  data-option-index={index}
                  disabled={option.disabled}
                  key={`${option.value}:${index}`}
                  tabIndex={-1}
                  onClick={() => chooseOption(index)}
                  onPointerMove={() => {
                    if (!option.disabled) {
                      setActiveIndex(index);
                    }
                  }}
                >
                  <span className="app-select-option-check" aria-hidden="true">
                    {index === selectedIndex ? (
                      <IconCheck size={14} stroke={2.2} />
                    ) : null}
                  </span>
                  <span>{option.label}</span>
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
