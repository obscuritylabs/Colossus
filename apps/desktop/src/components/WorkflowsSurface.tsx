import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { AutomationSurface, AutomationWelcome } from "@colossus/ui/automations";
import "@colossus/ui/styles/automations.css";
import {
  IconSparkles,
  IconTopologyStar3,
  IconPlayerPlay,
  IconCalendarTime,
} from "@tabler/icons-react";
import {
  getRegisteredWorkflow,
  listRegisteredWorkflows,
  listWorkflowRuns,
  workflowContext,
} from "../api";
import {
  occurrence,
  workflowFailure,
  type RegisteredWorkflow,
  type WorkflowContext,
  type WorkflowRun,
} from "../workflows";
import { WorkflowImport } from "./WorkflowImport";
import { WorkflowLogicDialog } from "./WorkflowLogicDialog";
import { WorkflowRunCreate } from "./WorkflowRunCreate";
import { ScheduledRunDetail } from "./ScheduledRunDetail";
import { ScheduleCreate } from "./ScheduleCreate";
import "./workflows.css";

export function WorkflowsSurface({
  targetId,
  workspaceName,
  runtimeReady,
  onCreateWithAgent,
  agentStarting,
}: {
  targetId: string | null;
  workspaceName: string;
  runtimeReady: boolean;
  onCreateWithAgent: (prompt: string) => void;
  agentStarting: boolean;
}) {
  const [context, setContext] = useState<WorkflowContext | null>(null);
  const [items, setItems] = useState<RegisteredWorkflow[]>([]);
  const [after, setAfter] = useState<string | null>(null);
  const [detail, setDetail] = useState<RegisteredWorkflow | null>(null);
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [runAfter, setRunAfter] = useState<string | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<
    "import" | "graph" | "run" | "schedule" | null
  >(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const alive = useRef(true);
  const generation = useRef(0);
  const load = useCallback(async () => {
    const current = ++generation.current;
    setDetail(null);
    setDialog(null);
    setRunId(null);
    setRuns([]);
    setContext(null);
    setItems([]);
    setError("");
    if (!targetId || !runtimeReady) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const value = await workflowContext(targetId);
      const page = value.workflows_read
        ? await listRegisteredWorkflows(targetId, value.selection_epoch, null)
        : { items: [], next_cursor: null };
      if (alive.current && current === generation.current) {
        setContext(value);
        setItems(page.items);
        setAfter(page.next_cursor);
      }
    } catch (error) {
      if (alive.current && current === generation.current)
        setError(workflowFailure(error));
    } finally {
      if (alive.current && current === generation.current) setLoading(false);
    }
  }, [targetId, runtimeReady]);
  useEffect(() => {
    alive.current = true;
    void load();
    return () => {
      alive.current = false;
      generation.current++;
    };
  }, [load]);
  async function inspect(workflowId: string) {
    if (!targetId || !context) return;
    const current = ++generation.current;
    setBusy(true);
    setError("");
    setDetail(null);
    setRuns([]);
    setRunId(null);
    setRunAfter(null);
    try {
      const value = await getRegisteredWorkflow(
        targetId,
        context.selection_epoch,
        workflowId,
      );
      if (alive.current && current === generation.current) setDetail(value);
      if (context.workflow_runs_read && context.workflow_run_history) {
        const page = await listWorkflowRuns(
          targetId,
          context.selection_epoch,
          workflowId,
          null,
        );
        if (alive.current && current === generation.current) {
          setRuns(page.items);
          setRunAfter(page.next_cursor);
        }
      }
    } catch (error) {
      if (alive.current && current === generation.current)
        setError(workflowFailure(error));
    } finally {
      if (alive.current && current === generation.current) setBusy(false);
    }
  }
  async function more(history: boolean) {
    if (!targetId || !context || (history && !detail)) return;
    const current = generation.current;
    setBusy(true);
    setError("");
    try {
      if (history && detail) {
        const page = await listWorkflowRuns(
          targetId,
          context.selection_epoch,
          detail.workflow_id,
          runAfter,
        );
        if (alive.current && current === generation.current) {
          setRuns((items) => [...items, ...page.items]);
          setRunAfter(page.next_cursor);
        }
      } else {
        const page = await listRegisteredWorkflows(
          targetId,
          context.selection_epoch,
          after,
        );
        if (alive.current && current === generation.current) {
          setItems((items) => [...items, ...page.items]);
          setAfter(page.next_cursor);
        }
      }
    } catch (error) {
      if (alive.current && current === generation.current)
        setError(workflowFailure(error));
    } finally {
      if (alive.current && current === generation.current) setBusy(false);
    }
  }
  const createWithAgent = () =>
    onCreateWithAgent(`@colossus/coding
Help me create a reusable workflow for this workspace.

Ask what to automate, the inputs it needs, and the result I want. Follow workspace conventions, permissions, and approval rules.
Review the steps, explicit tool capabilities, execution limits, and strict input schema with me. Prepare Colossus workflow YAML and help me import it into the library. Show how to test it with a manual run before scheduling.`);
  return (
    <AutomationSurface className="workflow-surface" label="Workspace workflows">
      <header className="workflow-page-header">
        <div>
          <p className="surface-breadcrumb">Workspace / {workspaceName}</p>
          <h2>Workflows</h2>
          <p>Reusable steps for recurring work.</p>
        </div>
        <div className="workflow-actions">
          <button
            className="button secondary"
            disabled={loading || busy || !runtimeReady}
            onClick={() => void load()}
          >
            Refresh
          </button>
          {items.length > 0 && (
            <Button
              variant="primary"
              disabled={
                loading ||
                busy ||
                agentStarting ||
                !runtimeReady ||
                !context?.workflows_register
              }
              onClick={createWithAgent}
            >
              <IconSparkles size={16} aria-hidden="true" /> Create with agent
            </Button>
          )}
          <button
            className="button secondary"
            disabled={loading || busy || !context?.workflows_register}
            onClick={() => setDialog("import")}
          >
            Import workflow
          </button>
        </div>
      </header>
      {message && <p role="status">{message}</p>}
      {error && <p role="alert">{error}</p>}
      {!runtimeReady ? (
        <div className="workflow-empty">
          Connect the selected Workspace to browse workflows.
        </div>
      ) : loading ? (
        <p role="status">Loading workflows…</p>
      ) : !context?.workflows_read ? (
        <div className="workflow-empty">
          This runtime does not advertise workflow discovery. Ask its
          administrator to enable the workflow read scope.
        </div>
      ) : error && !items.length ? null : !items.length ? (
        <AutomationWelcome
          title="Turn a repeatable task into a workflow"
          description="Describe what to automate. An agent will help you define the steps and inputs, then prepare a workflow you can run again."
          icon={<IconTopologyStar3 size={24} stroke={1.6} aria-hidden="true" />}
          action={
            <Button
              variant="primary"
              disabled={agentStarting || !context.workflows_register}
              onClick={createWithAgent}
            >
              <IconSparkles size={16} aria-hidden="true" /> Create with agent
            </Button>
          }
          features={[
            {
              title: "Define the steps",
              description: "Connect agent tasks, tools, and decisions.",
              icon: <IconTopologyStar3 size={20} aria-hidden="true" />,
            },
            {
              title: "Try a run",
              description: "Inspect the logic and review the results.",
              icon: <IconPlayerPlay size={20} aria-hidden="true" />,
            },
            {
              title: "Put it on a schedule",
              description: "Repeat the same workflow on your timing.",
              icon: <IconCalendarTime size={20} aria-hidden="true" />,
            },
          ]}
        />
      ) : (
        <div
          className={`workflow-layout ${runId ? "workflow-layout-has-run" : ""}`}
        >
          <div>
            <ul className="workflow-list">
              {items.map((item) => (
                <li key={item.workflow_id}>
                  <button
                    disabled={busy}
                    aria-pressed={detail?.workflow_id === item.workflow_id}
                    onClick={() => void inspect(item.workflow_id)}
                  >
                    <strong>{item.name}</strong>
                    <span>Version {item.version}</span>
                    <span>{item.description}</span>
                  </button>
                </li>
              ))}
            </ul>
            {after && (
              <button
                className="button secondary"
                disabled={busy}
                onClick={() => void more(false)}
              >
                Load more workflows
              </button>
            )}
          </div>
          <article className="workflow-detail">
            {!detail ? (
              <>
                <h3>Explore a workflow</h3>
                <p>
                  Select a definition to inspect its graph, inputs, and run
                  history.
                </p>
              </>
            ) : (
              <>
                <div className="workflow-detail-header">
                  <div>
                    <h3>{detail.name}</h3>
                    <p>{detail.description}</p>
                  </div>
                  <span className="workflow-status">{detail.version}</span>
                </div>
                <div className="workflow-actions">
                  <button
                    className="button secondary"
                    onClick={() => {
                      setRunId(null);
                      setDialog("graph");
                    }}
                  >
                    View workflow logic
                  </button>
                  <button
                    className="button primary"
                    disabled={
                      !context.workflow_runs_start ||
                      !detail.scheduling_eligible
                    }
                    onClick={() => setDialog("run")}
                  >
                    Run workflow
                  </button>
                  <button
                    className="button secondary"
                    disabled={
                      !context.schedules_create || !detail.scheduling_eligible
                    }
                    onClick={() => setDialog("schedule")}
                  >
                    Schedule workflow
                  </button>
                </div>
                {detail.unavailable_reason && (
                  <p role="alert">{detail.unavailable_reason}</p>
                )}
                <dl className="workflow-facts">
                  <dt>Definition</dt>
                  <dd>{detail.workflow_id}</dd>
                  <dt>Pinned hash</dt>
                  <dd>
                    <code>{detail.workflow_hash}</code>
                  </dd>
                </dl>
                <details>
                  <summary>Input schema</summary>
                  <pre className="workflow-json">
                    {JSON.stringify(detail.input_schema, null, 2)}
                  </pre>
                </details>
                <section
                  className="workflow-history"
                  aria-label="Workflow run history"
                >
                  <div className="workflow-detail-header">
                    <h4>Run history</h4>
                    <button
                      className="button secondary compact"
                      disabled={busy}
                      onClick={() => void inspect(detail.workflow_id)}
                    >
                      Refresh history
                    </button>
                  </div>
                  {!context.workflow_run_history ? (
                    <p>Run history is unavailable on this runtime.</p>
                  ) : !runs.length ? (
                    <p>No runs yet. Start a run or attach a schedule.</p>
                  ) : (
                    <ul className="workflow-list">
                      {runs.map((run) => (
                        <li key={run.run_id}>
                          <button
                            aria-pressed={runId === run.run_id}
                            onClick={() => setRunId(run.run_id)}
                          >
                            <strong>{run.status}</strong>
                            <span>{occurrence(run.created_at)}</span>
                            <span>{run.run_id}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                  {runAfter && (
                    <button
                      className="button secondary"
                      disabled={busy}
                      onClick={() => void more(true)}
                    >
                      Load older runs
                    </button>
                  )}
                </section>
              </>
            )}
          </article>
          {runId && targetId && context && (
            <aside
              className="workflow-output-panel"
              aria-label="Selected run output"
            >
              <button
                className="button secondary compact"
                onClick={() => setRunId(null)}
              >
                Close output
              </button>
              <ScheduledRunDetail
                key={runId}
                targetId={targetId}
                context={context}
                runId={runId}
              />
              <button
                className="button secondary"
                onClick={() => setDialog("graph")}
              >
                View run in graph
              </button>
            </aside>
          )}
        </div>
      )}
      {targetId && context && dialog === "import" && (
        <WorkflowImport
          targetId={targetId}
          context={context}
          onClose={() => setDialog(null)}
          onRegistered={() => {
            setMessage("Workflow registered.");
            void load();
          }}
        />
      )}
      {targetId && context && detail && dialog === "graph" && (
        <WorkflowLogicDialog
          targetId={targetId}
          context={context}
          workflowId={detail.workflow_id}
          expectedHash={
            runId
              ? runs.find((run) => run.run_id === runId)?.workflow_hash ||
                detail.workflow_hash
              : detail.workflow_hash
          }
          runId={runId}
          onClose={() => setDialog(null)}
        />
      )}
      {targetId && context && detail && dialog === "run" && (
        <WorkflowRunCreate
          targetId={targetId}
          context={context}
          workflow={detail}
          onClose={() => setDialog(null)}
          onStarted={(run) => {
            setDialog(null);
            setRuns((items) => [run, ...items]);
            setRunId(run.run_id);
            setMessage("Workflow run created.");
          }}
        />
      )}
      {targetId && context && detail && dialog === "schedule" && (
        <ScheduleCreate
          targetId={targetId}
          context={context}
          initialWorkflowId={detail.workflow_id}
          onClose={() => setDialog(null)}
          onCreated={(schedule) => {
            setDialog(null);
            setMessage(
              `Schedule ${schedule.record.schedule_id} created. Manage it in Schedules.`,
            );
          }}
        />
      )}
    </AutomationSurface>
  );
}
