import { Button } from "@colossus/ui";
import { Button as LinkButton } from "@colossus/ui/components/ui/button";
import { useEffect, useRef, useState } from "react";
import {
  IconArrowLeft,
  IconPlayerStop,
  IconTerminal2,
  IconLoader2,
} from "@tabler/icons-react";
import {
  request,
  awaitReceipt,
  projectPath,
  taskTitle,
  taskStatus,
  statusLabel,
  terminalStatuses,
  visibleOutput,
  type Task,
  type Update,
  type Permission,
  type CommandReceipt,
} from "./api";
import { Interactions } from "./Interactions";
export function TaskDetail({
  initial,
  project,
  permissions,
  nodeLabel,
  onBack,
  backHref,
}: {
  initial: Task;
  project: string;
  permissions: Permission[];
  nodeLabel: string;
  onBack: () => void;
  backHref?: string;
}) {
  const [task, setTask] = useState(initial),
    [updates, setUpdates] = useState<Update[]>([]),
    [error, setError] = useState(""),
    [connected, setConnected] = useState(false),
    [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const path = `${projectPath(project)}/tasks/${initial.task_id}`;
  const cancellation = useRef(crypto.randomUUID());
  async function refresh() {
    try {
      const response = await request<{ task: Task }>(path);
      setTask(response.task);
    } catch (error) {
      setError(error instanceof Error ? error.message : "Task refresh failed.");
    }
  }
  useEffect(() => {
    const abort = new AbortController();
    let alive = true;
    const refresh = () =>
      void request<{ task: Task }>(path, undefined, abort.signal)
        .then((response) => {
          if (alive) setTask(response.task);
        })
        .catch((error) => {
          if (alive)
            setError(
              error instanceof Error ? error.message : "Task refresh failed.",
            );
        });
    refresh();
    const timer = setInterval(refresh, 2000);
    return () => {
      alive = false;
      abort.abort();
      clearInterval(timer);
    };
  }, [path]);
  useEffect(() => {
    const source = new EventSource(`${path}/events?after=0`);
    source.onopen = () => setConnected(true);
    source.onerror = () => setConnected(false);
    source.addEventListener("run_update", (event) => {
      try {
        const update = JSON.parse(
          (event as MessageEvent<string>).data,
        ) as Update;
        setUpdates((current) =>
          current.some((existing) => existing.sequence === update.sequence)
            ? current
            : current.concat(update).sort((a, b) => a.sequence - b.sequence),
        );
      } catch {
        setError("The event stream returned an invalid update.");
        source.close();
      }
    });
    return () => source.close();
  }, [path]);
  const status = taskStatus(task),
    finished = terminalStatuses.has(status),
    output = visibleOutput(updates);
  async function cancel() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const { command } = await request<{ command: CommandReceipt }>(
        `${path}/cancel`,
        { mutation_id: cancellation.current },
      );
      const settled = await awaitReceipt(path, command);
      setNotice(
        settled
          ? "Cancellation confirmed by the runtime."
          : "Cancellation queued. It will be delivered when the runtime reconnects; retrying uses the same request.",
      );
      await refresh();
    } catch (error) {
      setError(error instanceof Error ? error.message : "Cancellation failed.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="detail">
      {backHref ? (
        <LinkButton asChild variant="ghost" className="back">
          <a
            href={backHref}
            onClick={(event) => {
              if (
                event.defaultPrevented ||
                event.button !== 0 ||
                event.metaKey ||
                event.ctrlKey ||
                event.shiftKey ||
                event.altKey
              )
                return;
              event.preventDefault();
              onBack();
            }}
          >
            <IconArrowLeft size={17} aria-hidden="true" />
            All tasks
          </a>
        </LinkButton>
      ) : (
        <Button variant="tertiary" className="back" onClick={onBack}>
          <IconArrowLeft size={17} />
          All tasks
        </Button>
      )}
      <div className="detail-header">
        <div>
          <div className="eyebrow">
            {nodeLabel} <span> / </span> {task.request.mode}
          </div>
          <h2>{taskTitle(task)}</h2>
        </div>
        <span className={`status status-${status}`}>{statusLabel(status)}</span>
      </div>
      <div className="detail-meta">
        <span>
          <span className={`dot ${connected ? "live" : ""}`} />
          {connected ? "Live event stream" : "Reconnecting event stream"}
        </span>
        <span>{task.request.role}</span>
        <span title={task.task_id}>Task {task.task_id.slice(0, 8)}</span>
        <Button
          variant="secondary"
          disabled={
            busy || finished || !task.run_id || !permissions.includes("control")
          }
          onClick={() => void cancel()}
        >
          <IconPlayerStop size={15} />
          {busy ? "Requesting…" : "Cancel task"}
        </Button>
      </div>
      {error && (
        <div role="alert" className="alert">
          {error}
          <Button aria-label="Dismiss error" onClick={() => setError("")}>
            ×
          </Button>
        </div>
      )}
      {notice && (
        <p className="muted" role="status">
          {notice}
        </p>
      )}
      {task.dispatch_error && (
        <div className="alert" role="alert">
          {task.dispatch_error.message}
        </div>
      )}
      {task.output_limited && (
        <div className="alert" role="status">
          This task reached the cloud detail limit. Its status and controls
          remain available. Earlier output is retained here; open the run
          locally for complete output or any interactions too large to display
          here.
        </div>
      )}
      <Interactions
        interactions={task.snapshot?.pending_interactions ?? []}
        project={project}
        task={task.task_id}
        permissions={permissions}
        onUpdate={() => void refresh()}
        onError={setError}
      />
      <section className="output-panel">
        <div className="panel-heading">
          <IconTerminal2 size={18} />
          <h2>Run output</h2>
          <span className="muted">{updates.length} events</span>
        </div>
        {output ? (
          <pre className="run-output">{output}</pre>
        ) : (
          <div className="output-empty">
            {finished ? (
              <IconTerminal2 size={30} />
            ) : (
              <IconLoader2 size={26} className="spin" />
            )}
            <h3>
              {task.run_id
                ? finished
                  ? "No text output was released"
                  : "The runtime is working"
                : "Waiting for the runtime"}
            </h3>
            <p>
              {task.run_id
                ? "Released output will appear here as the task progresses."
                : "This task stays assigned to its selected runtime and starts when the connection is ready."}
            </p>
          </div>
        )}
        <details className="event-log">
          <summary>
            Event activity <span>{updates.length}</span>
          </summary>
          <ol>
            {updates.map((update) => (
              <li key={update.sequence}>
                <span className="mono">{update.sequence}</span>
                <strong>
                  {statusLabel(Object.keys(update.update)[0] ?? "update")}
                </strong>
                <time>{new Date(update.created_at).toLocaleTimeString()}</time>
              </li>
            ))}
          </ol>
        </details>
      </section>
    </div>
  );
}
