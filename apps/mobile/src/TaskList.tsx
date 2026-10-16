import { HostPlatformIcon } from "@colossus/ui";
import { IconChevronRight } from "@tabler/icons-react";
import type { FleetNode, Host, Task } from "@colossus/control-plane-client";
import { TaskStatus } from "./TaskStatus";

export function TaskList({
  tasks,
  nodes,
  hosts,
  query,
  onSelect,
}: {
  tasks: Task[];
  nodes: FleetNode[];
  hosts: Map<string, Host>;
  query: string;
  onSelect: (task: Task) => void;
}) {
  const filtered = tasks.filter((task) =>
    taskTitle(task).toLocaleLowerCase().includes(query.toLocaleLowerCase()),
  );
  return (
    <div className="mobile-rows">
      {filtered.map((task) => {
        const node = nodes.find((node) => node.node.node_id === task.node_id);
        const host = hosts.get(node?.node.host_id ?? "");
        return (
          <button
            type="button"
            className="mobile-row mobile-task-row"
            key={task.task_id}
            onClick={() => onSelect(task)}
          >
            <HostPlatformIcon
              platform={host?.platform ?? "unknown"}
              size={25}
            />
            <span>
              <strong>{taskTitle(task)}</strong>
              <small>
                {[host?.label ?? node?.node.label, node?.node.workspace_label]
                  .filter(Boolean)
                  .join(" · ") || "Assigned workspace"}
              </small>
              <TaskStatus task={task} />
            </span>
            <IconChevronRight size={18} aria-hidden="true" />
          </button>
        );
      })}
      {!filtered.length && (
        <div className="mobile-empty">
          <h2>{query ? "No matching tasks" : "No tasks yet"}</h2>
          <p>
            {query
              ? "Try another search in the loaded task pages."
              : "Start a conversation to create your first task."}
          </p>
        </div>
      )}
    </div>
  );
}
export function taskTitle(task: Task) {
  // Runtime titles may describe the whole session; task rows identify this turn.
  return (
    task.request.input
      .map((part) => part.text)
      .join(" ")
      .trim()
      .slice(0, 100) ||
    task.snapshot?.run.title ||
    "Untitled task"
  );
}
