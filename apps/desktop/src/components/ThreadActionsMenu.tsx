import { useRef, useState } from "react";
import {
  IconArchive,
  IconDots,
  IconGitFork,
  IconLoader2,
  IconPencil,
  IconPin,
} from "@tabler/icons-react";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@colossus/ui/components/ui/dropdown-menu";
import type { Run } from "../types";
import { isTerminalStatus } from "../types";
import { canForkThread, isThreadForkDraft } from "../thread-fork";
import { runModeLabel, shortDateLabel } from "../presenters";
import "./thread-fork.css";

export function ThreadActionsMenu({
  run,
  title,
  pinned,
  disabled,
  localDisabled,
  archiving,
  onRename,
  onPin,
  onArchive,
  onFork,
}: {
  run: Run;
  title: string;
  pinned: boolean;
  disabled: boolean;
  localDisabled: boolean;
  archiving: boolean;
  onRename: () => void;
  onPin: () => void;
  onArchive: () => void;
  onFork?: (() => void) | undefined;
}) {
  const forkSelected = useRef(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [container, setContainer] = useState<HTMLElement | null>(null);
  return (
    <DropdownMenu
      modal={false}
      onOpenChange={(open) => {
        if (open) {
          // Keep the menu inside navigation's focus and pointer boundary.
          setContainer(
            triggerRef.current?.closest<HTMLElement>('[role="dialog"]') ?? null,
          );
        }
      }}
    >
      <DropdownMenuTrigger asChild>
        <button
          ref={triggerRef}
          type="button"
          className="work-item-action thread-actions-trigger"
          aria-label={`Thread actions for ${title}`}
          title="Thread actions"
        >
          <IconDots size={17} stroke={1.9} aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="thread-actions-content"
        container={container}
        collisionBoundary={container}
        collisionPadding={8}
        aria-label={`Actions for ${title}`}
        onEscapeKeyDown={(event) => event.stopPropagation()}
        onCloseAutoFocus={(event) => {
          if (forkSelected.current) {
            event.preventDefault();
            forkSelected.current = false;
          }
        }}
      >
        <DropdownMenuLabel className="thread-actions-context">
          <strong>{title}</strong>
          <span>
            {isThreadForkDraft(run) ? "Draft" : runModeLabel(run.mode)} ·{" "}
            <time dateTime={run.updatedAt}>
              {shortDateLabel(run.updatedAt)}
            </time>
          </span>
        </DropdownMenuLabel>
        <hr className="thread-actions-separator" />
        <DropdownMenuItem
          disabled={localDisabled}
          onSelect={onRename}
          aria-label={`Rename ${title}`}
        >
          <span className="thread-action-label">
            <IconPencil size={15} aria-hidden="true" /> Rename
          </span>
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={disabled || onFork === undefined || !canForkThread(run)}
          onSelect={() => {
            forkSelected.current = true;
            onFork?.();
          }}
          aria-label={`Fork ${title}`}
          title={
            isThreadForkDraft(run)
              ? "Send a message in this fork before forking it again"
              : !canForkThread(run)
                ? "Finish or cancel this thread before forking"
                : undefined
          }
        >
          <span className="thread-action-label">
            <IconGitFork size={15} aria-hidden="true" /> Fork thread
          </span>
        </DropdownMenuItem>
        <DropdownMenuCheckboxItem
          checked={pinned}
          disabled={localDisabled}
          onSelect={onPin}
          aria-label={`${pinned ? "Unpin" : "Pin"} ${title}`}
        >
          <span className="thread-action-label">
            <IconPin size={15} aria-hidden="true" />
            {pinned ? "Unpin" : "Pin"}
          </span>
        </DropdownMenuCheckboxItem>
        <hr className="thread-actions-separator" />
        <DropdownMenuItem
          disabled={disabled || archiving || !isTerminalStatus(run.status)}
          onSelect={onArchive}
          aria-label={`${isThreadForkDraft(run) ? "Discard draft" : "Archive"} ${title}`}
        >
          <span className="thread-action-label">
            {archiving ? (
              <IconLoader2 size={15} className="spin-icon" aria-hidden="true" />
            ) : (
              <IconArchive size={15} aria-hidden="true" />
            )}
            {archiving
              ? "Archiving…"
              : isThreadForkDraft(run)
                ? "Discard draft"
                : "Archive"}
          </span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
