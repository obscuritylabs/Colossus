import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconCloud, IconRefresh } from "@tabler/icons-react";
import { ApiFailure, projectPath, request } from "./api";
import { LoadState, Metrics } from "./Home";
import { HostedRuns } from "./HostedRuns";
import { RouteLink } from "./navigation";
import { useResource, errorMessage } from "./resources";
import { chatConnectionsHref, globalHref, hostedAgentHref } from "./routes";
import {
  hostedStatus,
  type HostedInventory,
  type HostedTarget,
} from "./hosted-types";
import "./hosted.css";

export function Hosted({
  project,
  identity,
  permissions,
  node,
  chatHref,
  onProjectSettings,
}: {
  project: string;
  identity: string;
  permissions: string[];
  node?: string | undefined;
  chatHref: string;
  onProjectSettings: () => void;
}) {
  const resource = useResource<HostedInventory>(
    `${projectPath(project)}/hosted`,
    3000,
    identity,
  );
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const abort = useRef<AbortController | null>(null);
  useEffect(() => () => abort.current?.abort(), []);
  const target =
    !resource.error &&
    resource.data?.target?.node.project_id === project &&
    resource.data.target.node.owner_user_id === identity &&
    resource.data.target.execution.kind === "hosted"
      ? resource.data.target
      : null;
  const activity = resource.data?.activity;
  const canExecute = permissions.includes("execute");
  const canControl = canExecute && permissions.includes("control");
  const warm = target?.execution.state === "warm";
  const active = activity
    ? activity.running + activity.waiting + activity.queued
    : 0;
  const status = activity ? hostedStatus(activity) : "Status unavailable";

  async function changeRuntime(action: "load" | "release") {
    if (busy || !target) return;
    const controller = new AbortController();
    abort.current = controller;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await request(
        `${projectPath(project)}/hosted/runtime/${action}`,
        {},
        controller.signal,
      );
      if (controller.signal.aborted) return;
      setNotice(
        action === "load"
          ? "Runtime loaded. It can retire automatically when idle."
          : "Idle runtime released. New work will load it again.",
      );
    } catch (e) {
      if (controller.signal.aborted) return;
      setError(
        e instanceof ApiFailure && e.status === 409
          ? "The runtime is busy or work needs reconciliation. Review its runs and refresh before releasing it."
          : errorMessage(e),
      );
    } finally {
      if (!controller.signal.aborted) {
        setBusy(false);
        resource.refresh();
      }
    }
  }

  const columns: DataTableColumn<HostedTarget>[] = [
    {
      id: "agent",
      label: "Agent",
      value: (item) => item.node.label,
      rowHeader: true,
      hideable: false,
      cell: (item) => (
        <div className="catalog-identity">
          <span className="resource-icon">
            <IconCloud size={18} aria-hidden="true" />
          </span>
          <div>
            <RouteLink
              className="catalog-name"
              href={hostedAgentHref(project, item.node.node_id)}
            >
              {item.node.label}
            </RouteLink>
            <small>Your personal agent</small>
          </div>
        </div>
      ),
    },
    { id: "status", label: "Work", value: () => status },
    {
      id: "runtime",
      label: "Runtime",
      value: () => (warm ? "Loaded" : "On demand"),
    },
    {
      id: "roles",
      label: "Roles",
      value: (item) => item.node.roles.join(", "),
    },
  ];
  const unavailable =
    node &&
    !resource.loading &&
    !resource.error &&
    (!target || target.node.node_id !== node);
  return (
    <section className="control-page hosted-page">
      {node ? (
        <RouteLink href={globalHref("hosted", project)}>
          ← Hosted agents
        </RouteLink>
      ) : null}
      <header className="page-heading">
        <div>
          <h1>
            {node && target?.node.node_id === node
              ? target.node.label
              : "Hosted agents"}
          </h1>
          <p>
            {node
              ? "Your execution target for chat and tasks in this project."
              : "Agents hosted by Colossus, with execution loaded when work arrives."}
          </p>
        </div>
        <Button
          variant="tertiary"
          aria-label="Refresh hosted agents"
          disabled={busy}
          onClick={resource.refresh}
        >
          <IconRefresh size={16} aria-hidden="true" />
          Refresh
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {unavailable ? (
        <p className="alert" role="alert">
          This Hosted agent is unavailable in the selected project.
        </p>
      ) : null}
      {!node &&
      !resource.loading &&
      !resource.error &&
      !resource.data?.enabled ? (
        <div className="empty-state">
          <h2>Hosted agents are not enabled</h2>
          <p>Ask your operator to enable Hosted execution for this project.</p>
        </div>
      ) : null}
      {!resource.loading &&
      !resource.error &&
      resource.data?.enabled &&
      !target ? (
        <p className="alert" role="alert">
          The Hosted target could not be verified. Refresh to try again.
        </p>
      ) : null}
      {target && !node ? (
        <>
          <DataTable
            data={[target]}
            columns={columns}
            getRowId={(item) => item.node.node_id}
            label="Hosted agents"
            itemLabel="agents"
            search={{ columnId: "agent", label: "Search hosted agents" }}
          />
          <p className="field-help">
            You have one personal Hosted agent per project. Its conversations
            share this target and keep separate histories. Only your agent is
            shown here.
          </p>
        </>
      ) : null}
      {node && target?.node.node_id === node ? (
        <>
          <Metrics
            items={[
              { label: "Work", value: status },
              {
                label: "Running",
                value: `${activity?.running ?? "—"}${activity?.limited ? "+" : ""}`,
                note: "Includes cancellation and output reconciliation",
              },
              {
                label: "Waiting",
                value: `${activity?.waiting ?? "—"}${activity?.limited ? "+" : ""}`,
                note: "Open a run to respond",
              },
              {
                label: "Queued",
                value: `${activity?.queued ?? "—"}${activity?.limited ? "+" : ""}`,
              },
            ]}
          />
          {activity?.limited ? (
            <p className="field-help">
              Counts cover up to 100 unfinished runs and may be lower bounds.
            </p>
          ) : null}
          <section className="control-card" aria-label="Runtime lifecycle">
            <header>
              <h2>Runtime</h2>
              <span className="muted">{warm ? "Loaded" : "On demand"}</span>
            </header>
            <p className="field-help">
              Execution loads automatically for new work and retires when idle.
              Releasing an idle runtime keeps your agent, conversations and
              files. It also closes unfinished connection setup.
            </p>
            <div className="hosted-actions">
              <RouteLink
                className="ui-button ui-button--primary"
                href={chatHref}
              >
                Open chat
              </RouteLink>
              {canExecute ? (
                <Button
                  variant="secondary"
                  disabled={busy || warm || target.node.revoked}
                  onClick={() => void changeRuntime("load")}
                >
                  {busy ? "Updating runtime…" : "Load runtime"}
                </Button>
              ) : null}
              {canControl ? (
                <Button
                  variant="secondary"
                  disabled={
                    busy || !warm || !activity || active > 0 || activity.limited
                  }
                  onClick={() => void changeRuntime("release")}
                >
                  Release idle runtime
                </Button>
              ) : null}
            </div>
            {!canExecute ? (
              <p className="field-help">
                Execute permission is required to load this runtime or start
                work.
              </p>
            ) : null}
            {active > 0 ? (
              <p className="field-help">
                Active or waiting work keeps the runtime loaded. Open a run to
                respond or cancel.
              </p>
            ) : null}
            {error ? (
              <p className="alert" role="alert">
                {error}
              </p>
            ) : null}
            {notice ? (
              <p className="field-help" role="status">
                {notice}
              </p>
            ) : null}
          </section>
          <div className="dashboard-grid">
            <section className="control-card" aria-label="Hosted configuration">
              <header>
                <h2>Configuration</h2>
              </header>
              <dl className="stat-list">
                <div>
                  <dt>Owner</dt>
                  <dd>You</dd>
                </div>
                <div>
                  <dt>Roles</dt>
                  <dd>{target.node.roles.join(", ") || "None configured"}</dd>
                </div>
                <div>
                  <dt>Providers, models and MCP definitions</dt>
                  <dd>Project configuration</dd>
                </div>
              </dl>
              <div className="hosted-actions">
                {permissions.includes("administer") ? (
                  <Button variant="secondary" onClick={onProjectSettings}>
                    Project settings
                  </Button>
                ) : null}
                <RouteLink
                  className="ui-button ui-button--secondary"
                  href={chatConnectionsHref(project)}
                >
                  Your connections
                </RouteLink>
              </div>
            </section>
            <section className="control-card" aria-label="Execution access">
              <header>
                <h2>Execution access</h2>
              </header>
              <dl className="stat-list">
                <div>
                  <dt>Environment</dt>
                  <dd>Controlled Hosted runtime</dd>
                </div>
                <div>
                  <dt>Managed files</dt>
                  <dd>
                    {target.execution.managed_files
                      ? "Available"
                      : "Unavailable"}
                  </dd>
                </div>
                <div>
                  <dt>Filesystem access</dt>
                  <dd>
                    {target.execution.filesystem ? "Available" : "Unavailable"}
                  </dd>
                </div>
                <div>
                  <dt>Process execution</dt>
                  <dd>
                    {target.execution.process ? "Available" : "Unavailable"}
                  </dd>
                </div>
              </dl>
            </section>
          </div>
          <HostedRuns
            project={project}
            node={target.node.node_id}
            identity={identity}
          />
        </>
      ) : null}
    </section>
  );
}
