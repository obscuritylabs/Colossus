import { lazy, Suspense, useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import {
  IconActivity,
  IconArrowRight,
  IconMessageCircle,
  IconRefresh,
  IconServer,
} from "@tabler/icons-react";
import { useResource } from "./resources";
import {
  dashboard,
  type Activity,
  type Analytics,
  type Dashboard,
} from "./control-api";
import type { Thread } from "./api";
import { RouteLink } from "./navigation";
import { globalHref, threadHref } from "./routes";
const ActivityLineChart = lazy(() =>
  import("@colossus/ui/charts").then((module) => ({
    default: module.ActivityLineChart,
  })),
);
export function LoadState({
  loading,
  error,
  retry,
}: {
  loading: boolean;
  error: string;
  retry: () => void;
}) {
  return error ? (
    <div className="alert" role="alert">
      {error}
      <Button onClick={retry}>Retry</Button>
    </div>
  ) : loading ? (
    <p className="muted" role="status">
      Loading…
    </p>
  ) : null;
}
export function Metrics({
  items,
}: {
  items: { label: string; value: number | string; note?: string }[];
}) {
  return (
    <div className="metric-grid">
      {items.map((item) => (
        <div className="metric-card" key={item.label}>
          <span>{item.label}</span>
          <strong>
            {typeof item.value === "number"
              ? item.value.toLocaleString()
              : item.value}
          </strong>
          {item.note ? <small>{item.note}</small> : null}
        </div>
      ))}
    </div>
  );
}
export type ActivityDays = 7 | 30 | 90;
export function ActivityRange({
  days,
  onChange,
}: {
  days: ActivityDays;
  onChange: (days: ActivityDays) => void;
}) {
  return (
    <label className="activity-range-selector">
      <span className="sr-only">Activity time range</span>
      <DropdownSelect
        aria-label="Activity time range"
        value={String(days)}
        onChange={(event) =>
          onChange(Number(event.target.value) as ActivityDays)
        }
      >
        <option value="7">Last 7 days</option>
        <option value="30">Last 30 days</option>
        <option value="90">Last 90 days</option>
      </DropdownSelect>
    </label>
  );
}
export function ActivityChart({
  activity,
  days = 7,
  onDaysChange,
}: {
  activity: Activity[];
  days?: ActivityDays;
  onDaysChange?: ((days: ActivityDays) => void) | undefined;
}) {
  const totals = { runs: 0, completed: 0, failed: 0 };
  for (const day of activity) {
    totals.runs += day.runs;
    totals.completed += day.completed;
    totals.failed += day.failed;
  }
  return (
    <section className="control-card activity-line-card">
      <header className="activity-line-heading">
        <div>
          <h3>
            <IconActivity size={17} aria-hidden="true" />
            Run activity
          </h3>
          <p>Daily execution outcomes over the last {days} days.</p>
        </div>
        {onDaysChange ? (
          <ActivityRange days={days} onChange={onDaysChange} />
        ) : null}
      </header>
      {activity.length ? (
        <div className="activity-line-summary">
          {[
            { label: "Total runs", value: totals.runs },
            { label: "Completed", value: totals.completed },
            { label: "Failed", value: totals.failed },
          ].map((item) => (
            <div key={item.label}>
              <span>{item.label}</span>
              <strong>{item.value.toLocaleString()}</strong>
            </div>
          ))}
        </div>
      ) : null}
      <Suspense
        fallback={
          <p className="shared-chart-empty" role="status">
            Loading chart…
          </p>
        }
      >
        <ActivityLineChart data={activity} />
      </Suspense>
    </section>
  );
}
export function Home({
  onOpen,
  onFleet,
}: {
  onOpen: (thread: Thread) => void;
  onFleet: () => void;
}) {
  const [days, setDays] = useState<ActivityDays>(7);
  const resource = useResource<Dashboard>(dashboard(days), 15000),
    data = resource.data;
  return (
    <section className="control-page">
      <header className="page-heading">
        <div>
          <span className="eyebrow">Overview</span>
          <h1>Your Control Plane</h1>
          <p>
            Connected agents, active conversations, and work across your
            projects.
          </p>
        </div>
        <div className="activity-header-controls">
          {!data ? <ActivityRange days={days} onChange={setDays} /> : null}
          <Button onClick={resource.refresh}>
            <IconRefresh size={16} aria-hidden="true" />
            Refresh
          </Button>
        </div>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {data ? (
        <>
          <Metrics
            items={[
              {
                label: "Agents online",
                value: data.counts.online_agents,
                note: `of ${data.counts.agents} enrolled agents`,
              },
              { label: "Active runs", value: data.counts.active_runs },
              { label: "Queued tasks", value: data.counts.queued_tasks },
              { label: "Projects", value: data.counts.projects },
            ]}
          />
          <div className="dashboard-grid">
            <ActivityChart
              activity={data.activity}
              days={days}
              onDaysChange={setDays}
            />
            <section className="control-card">
              <header>
                <h3>
                  <IconServer size={17} aria-hidden="true" />
                  Fleet
                </h3>
                <RouteLink
                  className="ui-button ui-button--tertiary"
                  href={globalHref("fleet")}
                  onNavigate={onFleet}
                >
                  Open Fleet
                  <IconArrowRight size={16} aria-hidden="true" />
                </RouteLink>
              </header>
              <dl className="stat-list">
                <div>
                  <dt>Hosts</dt>
                  <dd>{data.counts.hosts}</dd>
                </div>
                <div>
                  <dt>Enrolled agents</dt>
                  <dd>{data.counts.agents}</dd>
                </div>
                <div>
                  <dt>Retained conversations</dt>
                  <dd>{data.counts.threads}</dd>
                </div>
                <div>
                  <dt>Failed runs · {days} days</dt>
                  <dd>{data.counts.failed_runs}</dd>
                </div>
              </dl>
              <p className="field-help">
                Run failures report execution outcomes. They are not policy
                violation counts.
              </p>
            </section>
          </div>
          <section className="control-card">
            <header>
              <h3>
                <IconMessageCircle size={17} aria-hidden="true" />
                Recent conversations
              </h3>
              <span className="muted">Across visible projects</span>
            </header>
            {data.recent_threads.length ? (
              <ul className="recent-list">
                {data.recent_threads.map(({ thread, project_name }) => (
                  <li key={thread.thread_id}>
                    <RouteLink
                      href={threadHref(thread.project_id, thread.thread_id)}
                      onNavigate={() => onOpen(thread)}
                    >
                      <span>
                        <strong>
                          {thread.title || "Untitled conversation"}
                        </strong>
                        <small>
                          {project_name} ·{" "}
                          {thread.source === "runtime"
                            ? "Shared local session"
                            : "Control Plane conversation"}
                        </small>
                      </span>
                      <time dateTime={thread.updated_at}>
                        {new Date(thread.updated_at).toLocaleDateString(
                          undefined,
                          { month: "short", day: "numeric" },
                        )}
                      </time>
                      <IconArrowRight size={16} aria-hidden="true" />
                    </RouteLink>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="empty-copy">
                Start a conversation from an agent in Fleet.
              </p>
            )}
          </section>
          {data.telemetry && !data.telemetry.complete ? (
            <p className="sync-notice">
              This overview covers a bounded set of retained records.{" "}
              {data.telemetry.description}
            </p>
          ) : null}
        </>
      ) : null}
    </section>
  );
}
export function AnalyticsPanel({ path }: { path: string }) {
  const [days, setDays] = useState<ActivityDays>(7);
  const resource = useResource<Analytics>(
      `${path}${path.includes("?") ? "&" : "?"}days=${days}`,
      15000,
    ),
    data = resource.data;
  return (
    <>
      <div className="analytics-range-toolbar">
        <ActivityRange days={days} onChange={setDays} />
      </div>
      <LoadState {...resource} retry={resource.refresh} />
      {data ? (
        <>
          <Metrics
            items={[
              { label: "Total runs", value: data.counts.runs },
              { label: "Completed", value: data.counts.completed },
              { label: "Failed", value: data.counts.failed },
              {
                label: "Active / queued",
                value: `${data.counts.active} / ${data.counts.queued}`,
              },
            ]}
          />
          <ActivityChart activity={data.activity} days={days} />
          <section className="control-card">
            <header>
              <h3>Released usage</h3>
              <span className="muted">Past {data.window_days} days</span>
            </header>
            <Metrics
              items={[
                {
                  label: "Input tokens",
                  value: data.usage.input_tokens ?? "Unavailable",
                },
                {
                  label: "Output tokens",
                  value: data.usage.output_tokens ?? "Unavailable",
                },
                {
                  label: "Estimated cost",
                  value:
                    data.usage.estimated_cost === null
                      ? "Unavailable"
                      : `${data.usage.currency ?? ""} ${data.usage.estimated_cost.toFixed(2)}`,
                },
              ]}
            />
            <p className="field-help">
              {data.usage.coverage ||
                "Usage includes only reliable provider values released by runtimes. Unavailable values are not zero."}
            </p>
          </section>
          {data.telemetry && !data.telemetry.complete ? (
            <p className="sync-notice">
              Analytics coverage is incomplete. {data.telemetry.description}
            </p>
          ) : null}
        </>
      ) : null}
    </>
  );
}
