import { ReleasedRunActivity } from "@colossus/ui/conversation";
import { taskStatus, type Task, type Update } from "./api";

export function ThreadRunActivity({
  task,
  updates,
  label,
}: {
  task: Task;
  updates: Update[];
  label: string;
}) {
  return (
    <ReleasedRunActivity
      status={taskStatus(task)}
      updates={updates}
      label={label}
      terminal={task.snapshot?.run.terminal}
      updatedAt={task.snapshot?.run.updated_at}
    />
  );
}
