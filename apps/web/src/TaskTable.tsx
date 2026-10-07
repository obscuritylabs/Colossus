import { useMemo } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import {
  IconArrowRight,
  IconListDetails,
  IconServer,
} from "@tabler/icons-react";
import {
  statusLabel,
  taskStatus,
  taskTitle,
  terminalStatuses,
  type FleetNode,
  type Task,
} from "./api";
import { RouteLink } from "./navigation";
import { globalHref, taskHref, threadHref } from "./routes";

const getRowId = (task: Task) => task.task_id;
const initialSorting = [{ id: "created", desc: true }];

export function TaskTable({
  tasks,
  nodes,
  loading,
  hasMore,
  onMore,
  onOpen,
  onFleet,
  project,
}: {
  tasks: Task[];
  nodes: FleetNode[];
  loading: boolean;
  hasMore: boolean;
  onMore: () => void;
  onOpen: (task: Task) => void;
  onFleet: () => void;
  project?: string;
}) {
  const columns = useMemo<DataTableColumn<Task>[]>(() => {
    const labels = new Map(
      nodes.map((fleet) => [fleet.node.node_id, fleet.node.label]),
    );
    const runtimeIds = [...new Set(tasks.map((task) => task.node_id))];
    const runtimeLabel = (id: string) => labels.get(id) ?? id.slice(0, 8);
    return [
      {
        id: "title",
        label: "Task",
        value: taskTitle,
        hideable: false,
        rowHeader: true,
        className: "task-title-cell",
        cell: (task) => (
          <div className="catalog-identity">
            <span className="resource-icon">
              <IconListDetails size={16} aria-hidden="true" />
            </span>
            <div>
              <RouteLink
                href={
                  task.thread_id
                    ? threadHref(task.project_id, task.thread_id)
                    : taskHref(task.project_id, task.task_id)
                }
                className="catalog-name"
                onNavigate={() => onOpen(task)}
              >
                {taskTitle(task)}
              </RouteLink>
              <small>
                {task.request.role} · {task.request.mode}
              </small>
            </div>
          </div>
        ),
      },
      {
        id: "runtime",
        label: "Runtime",
        value: (task) => runtimeLabel(task.node_id),
        cell: (task) => (
          <span className="catalog-connection">
            {runtimeLabel(task.node_id)}
          </span>
        ),
        filter: {
          label: "Filter by runtime",
          options: [
            { value: "", label: "All runtimes" },
            ...runtimeIds
              .sort((a, b) => runtimeLabel(a).localeCompare(runtimeLabel(b)))
              .map((id) => ({ value: id, label: runtimeLabel(id) })),
          ],
          matches: (task, value) => !value || task.node_id === value,
        },
      },
      {
        id: "status",
        label: "Status",
        value: (task) => statusLabel(taskStatus(task)),
        cell: (task) => (
          <span className={`status status-${taskStatus(task)}`}>
            {statusLabel(taskStatus(task))}
          </span>
        ),
        filter: {
          label: "Filter by status",
          options: [
            { value: "", label: "All statuses" },
            { value: "active", label: "Active" },
            { value: "finished", label: "Finished" },
          ],
          matches: (task, value) =>
            !value ||
            (value === "active"
              ? !terminalStatuses.has(taskStatus(task))
              : terminalStatuses.has(taskStatus(task))),
        },
      },
      {
        id: "created",
        label: "Created",
        value: (task) =>
          Date.parse(task.created_at || task.snapshot?.run.created_at || "") ||
          0,
        cell: (task) => {
          const created = task.created_at || task.snapshot?.run.created_at;
          return created ? (
            <time dateTime={created} title={new Date(created).toLocaleString()}>
              {new Date(created).toLocaleDateString(undefined, {
                month: "short",
                day: "numeric",
              })}
            </time>
          ) : (
            "Created time unavailable"
          );
        },
      },
      {
        id: "actions",
        label: "Actions",
        value: () => "",
        sortable: false,
        hideable: false,
        className: "catalog-row-actions",
        cell: (task) => (
          <RouteLink
            href={
              task.thread_id
                ? threadHref(task.project_id, task.thread_id)
                : taskHref(task.project_id, task.task_id)
            }
            className="ui-icon-button"
            aria-label={`Open task: ${taskTitle(task)}`}
            onNavigate={() => onOpen(task)}
          >
            <IconArrowRight size={16} aria-hidden="true" />
          </RouteLink>
        ),
      },
    ];
  }, [nodes, tasks, onOpen]);
  return (
    <DataTable
      key="project-tasks"
      data={tasks}
      columns={columns}
      getRowId={getRowId}
      label="Project tasks"
      itemLabel="tasks"
      initialSorting={initialSorting}
      search={{ columnId: "title", label: "Search tasks" }}
      loading={loading}
      empty={
        <div className="empty-state">
          <h3>{tasks.length ? "No matching tasks" : "No tasks yet"}</h3>
          <p>
            {tasks.length
              ? "Try another search or task filter."
              : nodes.some((fleet) => !fleet.node.revoked)
                ? "Describe a task above to start work on the selected runtime."
                : "Enroll a runtime to start your first task."}
          </p>
          {!nodes.some((fleet) => !fleet.node.revoked) && (
            <RouteLink
              className="ui-button ui-button--primary"
              href={globalHref("fleet", project)}
              onNavigate={onFleet}
            >
              <IconServer size={16} aria-hidden="true" />
              Set up a runtime
            </RouteLink>
          )}
        </div>
      }
      footer={
        hasMore && (
          <div className="task-history-footer">
            <p>Sorting and filters apply to the {tasks.length} loaded tasks.</p>
            <Button onClick={onMore} disabled={loading}>
              Load more history
            </Button>
          </div>
        )
      }
    />
  );
}
