import {
  IconCheck,
  IconChevronDown,
  IconLayoutSidebarRight,
} from "@tabler/icons-react";
import { useRef, useState, type ComponentType, type RefObject } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@colossus/ui/components/ui/dropdown-menu";
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
  disabled?: boolean;
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
  const localTrigger = useRef<HTMLButtonElement>(null);
  const trigger = triggerRef ?? localTrigger;
  const selected = options.find((option) => option.id === active);
  const Icon = pane && selected ? selected.icon : IconLayoutSidebarRight;
  if (options.length === 0) return null;
  return (
    <DropdownMenu open={open} onOpenChange={setOpen} modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          ref={trigger}
          className="button secondary compact tool-switcher-trigger"
          type="button"
          aria-label={pane ? "Switch pane tool" : "Open tools"}
        >
          <Icon size={17} aria-hidden />
          <span>{pane ? (selected?.label ?? "Tools") : "Tools"}</span>
          <IconChevronDown size={13} aria-hidden />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        className="tool-switcher-menu"
        aria-label="Workspace tools"
        aria-labelledby={undefined}
        align={pane ? "start" : "end"}
        sideOffset={7}
      >
        {options.map((option) => (
          <DropdownMenuItem
            key={option.id}
            className="tool-option"
            role="menuitemradio"
            aria-checked={active === option.id}
            disabled={option.disabled === true}
            onSelect={() => onSelect(option.id)}
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
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
