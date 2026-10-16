import type { Task } from "@colossus/control-plane-client";
import { statusLabel, taskStatus } from "@colossus/control-plane-client";
import {
  IconCircleCheck,
  IconCircleDot,
  IconClock,
  IconAlertCircle,
  IconCircleX,
} from "@tabler/icons-react";

export function TaskStatus({ task }: { task: Task }) {
  const state = taskStatus(task);
  const tone =
    state === "completed"
      ? "success"
      : ["failed", "interrupted", "outcome_unknown"].includes(state)
        ? "danger"
        : state === "waiting_for_input"
          ? "attention"
          : "muted";
  const Icon =
    state === "completed"
      ? IconCircleCheck
      : state === "queued"
        ? IconClock
        : tone === "danger" || tone === "attention"
          ? IconAlertCircle
          : state === "cancelled"
            ? IconCircleX
            : IconCircleDot;
  return (
    <span className="mobile-task-state" data-tone={tone}>
      <Icon size={15} aria-hidden="true" />
      {statusLabel(state)}
    </span>
  );
}
