import {
  IconCheck,
  IconChevronDown,
  IconLayoutSidebarRight,
} from "@tabler/icons-react";
import { useEffect, useId, useRef, useState } from "react";
import type { ComponentType, RefObject } from "react";
import "./tools.css";

export type WorkTool =
  | "files"
  | "artifacts"
  | "browser"
  | "terminal"
  | "shells"
  | "git"
  | "details"
  | "aside"
  | "research";
export interface ToolOption {
  id: WorkTool;
  label: string;
  description: string;
  icon: ComponentType<{ size?: number; "aria-hidden"?: boolean }>;
  count?: number;
}

export function ToolSwitcher({
  options,
  active,
  onSelect,
  pane = false,
  triggerRef,
}: {
  options: ToolOption[];
  active: WorkTool | null;
  onSelect: (tool: WorkTool) => void;
  pane?: boolean;
  triggerRef?: RefObject<HTMLButtonElement | null>;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const localTrigger = useRef<HTMLButtonElement>(null);
  const trigger = triggerRef ?? localTrigger;
  const id = useId();
  const selected = options.find((option) => option.id === active);
  const Icon = pane && selected ? selected.icon : IconLayoutSidebarRight;
  useEffect(() => {
    if (!open) return;
    root.current
      ?.querySelector<HTMLButtonElement>(
        '[role="menuitemradio"][aria-checked="true"]',
      )
      ?.focus();
    if (
      !root.current?.contains(document.activeElement) ||
      document.activeElement === trigger.current
    )
      root.current
        ?.querySelector<HTMLButtonElement>('[role="menuitemradio"]')
        ?.focus();
    function outside(event: PointerEvent) {
      if (event.target instanceof Node && !root.current?.contains(event.target))
        setOpen(false);
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open, trigger]);
  if (options.length === 0) return null;
  return (
    <div
      className="tool-switcher"
      ref={root}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          trigger.current?.focus();
        } else if (
          ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)
        ) {
          event.preventDefault();
          if (!open) {
            setOpen(true);
            return;
          }
          const items = Array.from(
            root.current?.querySelectorAll<HTMLButtonElement>(
              '[role="menuitemradio"]',
            ) ?? [],
          );
          const current = items.indexOf(
            document.activeElement as HTMLButtonElement,
          );
          const index =
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? items.length - 1
                : (current +
                    (event.key === "ArrowUp" ? -1 : 1) +
                    items.length) %
                  items.length;
          items[index]?.focus();
        }
      }}
    >
      <button
        ref={trigger}
        className="button secondary compact tool-switcher-trigger"
        type="button"
        aria-label={pane ? "Switch pane tool" : "Open tools"}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon size={17} aria-hidden />
        <span>{pane ? (selected?.label ?? "Tools") : "Tools"}</span>
        <IconChevronDown size={13} aria-hidden />
      </button>
      <div
        hidden={!open}
        className="tool-switcher-menu"
        role="menu"
        id={id}
        aria-label="Workspace tools"
      >
        {options.map((option) => (
          <button
            key={option.id}
            type="button"
            role="menuitemradio"
            aria-checked={active === option.id}
            onClick={() => {
              onSelect(option.id);
              setOpen(false);
              trigger.current?.focus();
            }}
          >
            <option.icon size={18} aria-hidden />
            <span>
              <strong>
                {option.label}
                {option.count === undefined ? null : (
                  <span className="tool-count">{option.count}</span>
                )}
              </strong>
              <small>{option.description}</small>
            </span>
            {active === option.id ? <IconCheck size={16} aria-hidden /> : null}
          </button>
        ))}
      </div>
    </div>
  );
}
