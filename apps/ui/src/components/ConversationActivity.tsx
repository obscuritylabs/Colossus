import type { ReactNode } from "react";
import {
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
} from "@tabler/icons-react";
import colossusMark from "../../assets/colossus-mark.svg";

export type ConversationActivityTone =
  "success" | "warning" | "danger" | "active" | "neutral";

export interface ConversationActivityProps {
  description: string;
  statusLabel: string;
  tone: ConversationActivityTone;
  open?: boolean | undefined;
  exceptionCount?: number;
  ariaLabel?: string;
  className?: string;
  children: ReactNode;
}

/** Hosts retain run identity, released activity, and all interaction authority. */
export function ConversationActivity({
  description,
  statusLabel,
  tone,
  open,
  exceptionCount = 0,
  ariaLabel = "Run activity",
  className = "",
  children,
}: ConversationActivityProps) {
  return (
    <details
      className={`run-activity run-activity-thread shared-conversation-activity ${className}`}
      data-tone={tone}
      aria-label={ariaLabel}
      open={open}
    >
      <summary className="run-activity-summary">
        <span className="run-activity-chevron" aria-hidden="true">
          <IconChevronDown size={16} stroke={1.9} />
        </span>
        <span className="run-activity-mark" aria-hidden="true">
          <img src={colossusMark} alt="" width={17} height={17} />
        </span>
        <span className="run-activity-title">
          <strong>Colossus</strong>
          <small>{description}</small>
        </span>
        <span className={`run-activity-status tone-${tone}`}>
          {tone === "success" ? (
            <IconCheck size={15} stroke={2} aria-hidden="true" />
          ) : tone === "danger" ? (
            <IconAlertTriangle size={15} stroke={1.9} aria-hidden="true" />
          ) : null}
          {statusLabel}
        </span>
        {exceptionCount > 0 ? (
          <span
            className="run-activity-exceptions"
            aria-label={`${exceptionCount} failed ${exceptionCount === 1 ? "action" : "actions"}`}
          >
            <IconAlertTriangle size={14} stroke={1.8} aria-hidden="true" />
            {exceptionCount}
          </span>
        ) : null}
      </summary>
      <div className="run-activity-body">{children}</div>
    </details>
  );
}
