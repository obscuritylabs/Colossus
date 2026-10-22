import { useMemo, useState } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import {
  projectPath,
  statusLabel,
  taskStatus,
  taskTitle,
  type Task,
} from "./api";
import { LoadState } from "./Home";
import { RouteLink } from "./navigation";
import { useResource } from "./resources";
import { chatThreadHref, taskHref } from "./routes";

export function HostedRuns({
  project,
  node,
  identity,
}: {
  project: string;
  node: string;
  identity: string;
}) {
  const [active, setActive] = useState(true);
  return (
    <section className="control-card" aria-label="Agent runs">
      <header>
        <h2>Runs</h2>
        <nav className="section-tabs" aria-label="Run history views">
          <Button
            variant="tertiary"
            aria-current={active ? "page" : undefined}
            onClick={() => setActive(true)}
          >
            Active runs
          </Button>
          <Button
            variant="tertiary"
            aria-current={!active ? "page" : undefined}
            onClick={() => setActive(false)}
          >
            All runs
          </Button>
        </nav>
      </header>
      <RunPage
        key={`${identity}:${project}:${node}:${active}`}
        project={project}
        node={node}
        identity={identity}
        active={active}
      />
    </section>
  );
}

function RunPage({
  project,
  node,
  identity,
  active,
}: {
  project: string;
  node: string;
  identity: string;
  active: boolean;
}) {
  const [cursors, setCursors] = useState<string[]>([]);
  const params = new URLSearchParams({ active: String(active), limit: "25" });
  if (cursors.length) params.set("after", cursors[cursors.length - 1]!);
  const resource = useResource<{ tasks: Task[]; next_cursor: string | null }>(
    `${projectPath(project)}/hosted/tasks?${params}`,
    cursors.length ? 0 : 3000,
    identity,
  );
  const tasks = resource.error
    ? []
    : (resource.data?.tasks ?? []).filter(
        (task) => task.project_id === project && task.node_id === node,
      );
  const columns = useMemo<DataTableColumn<Task>[]>(
    () => [
      {
        id: "task",
        label: "Run",
        value: taskTitle,
        rowHeader: true,
        hideable: false,
        cell: (task) => (
          <div className="catalog-identity">
            <div>
              <RouteLink
                className="catalog-name"
                href={taskHref(project, task.task_id)}
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
        id: "status",
        label: "Status",
        value: (task) => statusLabel(taskStatus(task)),
        cell: (task) => (
          <span className={`status status-${taskStatus(task)}`}>
            {statusLabel(taskStatus(task))}
          </span>
        ),
      },
      {
        id: "created",
        label: "Created",
        value: (task) => task.created_at ?? "",
        cell: (task) =>
          task.created_at ? (
            <time dateTime={task.created_at}>
              {new Date(task.created_at).toLocaleString()}
            </time>
          ) : (
            "Unavailable"
          ),
      },
      {
        id: "conversation",
        label: "Conversation",
        value: (task) => task.thread_id ?? "",
        sortable: false,
        cell: (task) =>
          task.thread_id ? (
            <RouteLink href={chatThreadHref(project, task.thread_id)}>
              Open chat
            </RouteLink>
          ) : (
            "Standalone task"
          ),
      },
    ],
    [project],
  );
  return (
    <>
      <LoadState {...resource} retry={resource.refresh} />
      <DataTable
        data={tasks}
        columns={columns}
        getRowId={(task) => task.task_id}
        label={active ? "Active hosted runs" : "Hosted run history"}
        itemLabel="runs"
        loading={resource.loading}
        search={{ columnId: "task", label: "Search loaded runs" }}
        empty={
          active
            ? "No active runs. Start a chat to give your agent work."
            : "No runs on this page."
        }
      />
      {cursors.length || resource.data?.next_cursor ? (
        <nav className="hosted-actions" aria-label="Run history pages">
          <Button
            variant="secondary"
            disabled={!cursors.length || resource.loading}
            onClick={() => setCursors((current) => current.slice(0, -1))}
          >
            Previous runs
          </Button>
          <span className="field-help">
            Page {cursors.length + 1} · search covers this page
          </span>
          <Button
            variant="secondary"
            disabled={
              resource.loading ||
              Boolean(resource.error) ||
              !resource.data?.next_cursor
            }
            onClick={() => {
              if (resource.data?.next_cursor)
                setCursors((current) => [
                  ...current,
                  resource.data!.next_cursor!,
                ]);
            }}
          >
            More runs
          </Button>
        </nav>
      ) : null}
    </>
  );
}
