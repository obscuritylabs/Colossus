import {
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconClock,
  IconCopy,
  IconFilter,
  IconPlayerPause,
  IconPlayerPlay,
  IconRefresh,
  IconRobot,
  IconSearch,
  IconSettings,
  IconTool,
  IconUser,
  IconZoomIn,
  IconZoomOut,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent } from "react";

import type {
  ListSessionActivityRequest,
  SessionActivity,
  SessionActivityKind,
  SessionActivityLane,
  SessionActivityPage,
  SessionActivityStatus,
} from "../types";

const LIVE_INTERVAL_MS = 3_000;
const SEARCH_DEBOUNCE_MS = 250;
const PAGE_SIZE = 100;

type ActivityInspectorTab = "summary" | "input" | "result" | "timing";

interface SessionActivityViewProps {
  sourceRunId: string;
  available: boolean;
  loadPage: (
    request: ListSessionActivityRequest,
  ) => Promise<SessionActivityPage>;
}

interface ActivityFilters {
  lanes: SessionActivityLane[];
  kinds: SessionActivityKind[];
  statuses: SessionActivityStatus[];
}

interface ActivityGroup {
  key: string;
  runId: string | null;
  turn: number | null;
  activities: SessionActivity[];
  newestSequence: number;
  startedAt: string;
}

const EMPTY_FILTERS: ActivityFilters = { lanes: [], kinds: [], statuses: [] };

const LANES: ReadonlyArray<{ lane: SessionActivityLane; label: string }> = [
  { lane: "agent", label: "Agent" },
  { lane: "tools", label: "Tools" },
  { lane: "system", label: "System" },
];

const KIND_OPTIONS: ReadonlyArray<{
  value: SessionActivityKind;
  label: string;
}> = [
  { value: "user", label: "User" },
  { value: "assistant", label: "Assistant" },
  { value: "tool", label: "Tool" },
  { value: "system", label: "System" },
];

const STATUS_OPTIONS: ReadonlyArray<{
  value: SessionActivityStatus;
  label: string;
}> = [
  { value: "running", label: "Running" },
  { value: "waiting", label: "Waiting" },
  { value: "completed", label: "Completed" },
  { value: "failed", label: "Failed" },
  { value: "cancelled", label: "Cancelled" },
  { value: "outcome_unknown", label: "Outcome unknown" },
];

function mergeActivities(
  current: readonly SessionActivity[],
  incoming: readonly SessionActivity[],
): SessionActivity[] {
  const byId = new Map(
    current.map((activity) => [activity.activityId, activity]),
  );
  for (const activity of incoming) {
    const previous = byId.get(activity.activityId);
    if (
      previous === undefined ||
      activity.lastSequence >= previous.lastSequence
    ) {
      byId.set(activity.activityId, activity);
    }
  }
  return [...byId.values()].sort(
    (left, right) => right.firstSequence - left.firstSequence,
  );
}

function activityGroups(
  activities: readonly SessionActivity[],
): ActivityGroup[] {
  const groups = new Map<string, ActivityGroup>();
  for (const activity of activities) {
    const key = `${activity.runId ?? "session"}:${activity.turn ?? "run"}`;
    const group = groups.get(key) ?? {
      key,
      runId: activity.runId,
      turn: activity.turn,
      activities: [],
      newestSequence: activity.lastSequence,
      startedAt: activity.startedAt,
    };
    group.activities.push(activity);
    group.newestSequence = Math.max(
      group.newestSequence,
      activity.lastSequence,
    );
    if (Date.parse(activity.startedAt) < Date.parse(group.startedAt)) {
      group.startedAt = activity.startedAt;
    }
    groups.set(key, group);
  }
  return [...groups.values()]
    .map((group) => ({
      ...group,
      activities: group.activities.sort(
        (left, right) => left.firstSequence - right.firstSequence,
      ),
    }))
    .sort((left, right) => right.newestSequence - left.newestSequence);
}

function shortTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

function fullTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(date);
}

function durationLabel(durationMs: number | null): string {
  if (durationMs === null) {
    return "—";
  }
  if (durationMs < 1_000) {
    return `${durationMs} ms`;
  }
  if (durationMs < 60_000) {
    return `${(durationMs / 1_000).toFixed(durationMs < 10_000 ? 2 : 1)} s`;
  }
  const minutes = Math.floor(durationMs / 60_000);
  const seconds = Math.round((durationMs % 60_000) / 1_000);
  return `${minutes}m ${seconds}s`;
}

function groupLabel(group: ActivityGroup): string {
  if (group.turn !== null) {
    return `Turn ${group.turn}`;
  }
  return group.runId === null ? "Session" : "Run activity";
}

function activityIcon(activity: SessionActivity) {
  const props = { size: 15, stroke: 1.8, "aria-hidden": true } as const;
  if (activity.kind === "user") {
    return <IconUser {...props} />;
  }
  if (activity.kind === "assistant") {
    return <IconRobot {...props} />;
  }
  if (activity.kind === "tool") {
    return <IconTool {...props} />;
  }
  return <IconSettings {...props} />;
}

function toggleFilter<Value extends string>(
  values: readonly Value[],
  value: Value,
): Value[] {
  return values.includes(value)
    ? values.filter((candidate) => candidate !== value)
    : [...values, value];
}

function activityRequest(
  sourceRunId: string,
  query: string,
  filters: ActivityFilters,
  pageToken = "",
): ListSessionActivityRequest {
  return {
    sourceRunId,
    query,
    lanes: filters.lanes,
    kinds: filters.kinds,
    statuses: filters.statuses,
    pageSize: PAGE_SIZE,
    pageToken,
  };
}

function ActivityTimeline({
  activities,
  selectedId,
  zoom,
  onSelect,
}: {
  activities: readonly SessionActivity[];
  selectedId: string | null;
  zoom: number;
  onSelect: (activity: SessionActivity) => void;
}) {
  const range = useMemo(() => {
    const timestamps = activities
      .flatMap((activity) => [activity.startedAt, activity.completedAt])
      .filter((value): value is string => value !== null)
      .map(Date.parse)
      .filter(Number.isFinite);
    const latest =
      timestamps.length === 0 ? Date.now() : Math.max(...timestamps);
    const earliest =
      timestamps.length === 0 ? latest - 60_000 : Math.min(...timestamps);
    const fullSpan = Math.max(latest - earliest, 60_000);
    const visibleSpan = Math.max(fullSpan / zoom, 10_000);
    return { start: latest - visibleSpan, end: latest, span: visibleSpan };
  }, [activities, zoom]);
  const ticks = Array.from({ length: 7 }, (_, index) => {
    const time = range.start + (range.span * index) / 6;
    return {
      left: `${(index / 6) * 100}%`,
      label: shortTime(new Date(time).toISOString()),
    };
  });
  return (
    <section
      className="activity-timeline"
      aria-label="Session activity timeline"
    >
      {LANES.map(({ lane, label }) => (
        <div className="activity-timeline-lane" key={lane}>
          <span className="activity-timeline-label">
            {lane === "agent" ? (
              <IconRobot size={16} stroke={1.7} aria-hidden="true" />
            ) : lane === "tools" ? (
              <IconTool size={16} stroke={1.7} aria-hidden="true" />
            ) : (
              <IconSettings size={16} stroke={1.7} aria-hidden="true" />
            )}
            {label}
          </span>
          <div className="activity-timeline-track">
            {activities
              .filter((activity) => activity.lane === lane)
              .map((activity) => {
                const started = Date.parse(activity.startedAt);
                if (!Number.isFinite(started) || started < range.start) {
                  return null;
                }
                const completed = activity.completedAt
                  ? Date.parse(activity.completedAt)
                  : started;
                const left = ((started - range.start) / range.span) * 100;
                const width = Math.max(
                  ((Math.max(completed, started) - started) / range.span) * 100,
                  0.75,
                );
                return (
                  <button
                    className={`activity-timeline-block kind-${activity.kind}`}
                    data-selected={selectedId === activity.activityId}
                    key={activity.activityId}
                    style={{
                      left: `${left}%`,
                      width: `${Math.min(width, 100 - left)}%`,
                    }}
                    type="button"
                    aria-label={`${activity.title}, ${shortTime(activity.startedAt)}`}
                    title={activity.title}
                    onClick={() => onSelect(activity)}
                  />
                );
              })}
          </div>
        </div>
      ))}
      <div className="activity-timeline-axis" aria-hidden="true">
        {ticks.map((tick) => (
          <span key={tick.left} style={{ left: tick.left }}>
            {tick.label}
          </span>
        ))}
      </div>
    </section>
  );
}

function ActivityInspector({ activity }: { activity: SessionActivity | null }) {
  const [tab, setTab] = useState<ActivityInspectorTab>("summary");
  useEffect(() => setTab("summary"), [activity?.activityId]);
  if (activity === null) {
    return (
      <aside
        className="activity-inspector is-empty"
        aria-label="Activity inspector"
      >
        <IconClock size={22} stroke={1.5} aria-hidden="true" />
        <strong>Select an activity</strong>
        <p>Choose a timeline block or feed row to inspect released details.</p>
      </aside>
    );
  }
  const content = tab === "input" ? activity.input : activity.result;
  return (
    <aside className="activity-inspector" aria-label="Activity inspector">
      <header>
        <span className={`activity-kind-icon kind-${activity.kind}`}>
          {activityIcon(activity)}
        </span>
        <div>
          <strong>{activity.title}</strong>
          <span>{activity.status?.replaceAll("_", " ") ?? activity.kind}</span>
        </div>
      </header>
      <nav aria-label="Activity detail sections">
        {(["summary", "input", "result", "timing"] as const).map((value) => (
          <button
            type="button"
            key={value}
            aria-current={tab === value ? "page" : undefined}
            onClick={() => setTab(value)}
          >
            {value.charAt(0).toUpperCase() + value.slice(1)}
          </button>
        ))}
      </nav>
      <div className="activity-inspector-body">
        {tab === "summary" ? (
          <>
            <dl>
              <div>
                <dt>Status</dt>
                <dd className={`status-${activity.status ?? "none"}`}>
                  {activity.status?.replaceAll("_", " ") ?? "—"}
                </dd>
              </div>
              <div>
                <dt>Turn</dt>
                <dd>{activity.turn ?? "—"}</dd>
              </div>
              <div>
                <dt>Actor</dt>
                <dd>{activity.actor}</dd>
              </div>
              <div>
                <dt>Sequence</dt>
                <dd>
                  {activity.firstSequence === activity.lastSequence
                    ? activity.firstSequence
                    : `${activity.firstSequence}–${activity.lastSequence}`}
                </dd>
              </div>
            </dl>
            <section>
              <h4>Summary</h4>
              <p>{activity.summary || "No released summary."}</p>
            </section>
            {Object.keys(activity.attributes).length > 0 ? (
              <section>
                <h4>Released metadata</h4>
                <dl className="activity-attributes">
                  {Object.entries(activity.attributes).map(([key, value]) => (
                    <div key={key}>
                      <dt>{key.replaceAll("_", " ")}</dt>
                      <dd>{value}</dd>
                    </div>
                  ))}
                </dl>
              </section>
            ) : null}
          </>
        ) : tab === "timing" ? (
          <dl className="activity-timing">
            <div>
              <dt>Started</dt>
              <dd>{fullTime(activity.startedAt)}</dd>
            </div>
            <div>
              <dt>Completed</dt>
              <dd>
                {activity.completedAt === null
                  ? "—"
                  : fullTime(activity.completedAt)}
              </dd>
            </div>
            <div>
              <dt>Duration</dt>
              <dd>{durationLabel(activity.durationMs)}</dd>
            </div>
            <div>
              <dt>Canonical records</dt>
              <dd>{activity.sourceEventTypes.join(", ")}</dd>
            </div>
          </dl>
        ) : content === null ? (
          <div className="activity-inspector-empty-copy">
            No released {tab} is available for this activity.
          </div>
        ) : (
          <section className="activity-payload">
            <header>
              <span>{content.format.toUpperCase()}</span>
              <button
                type="button"
                aria-label={`Copy released ${tab}`}
                onClick={() =>
                  void navigator.clipboard.writeText(content.value)
                }
              >
                <IconCopy size={15} stroke={1.7} aria-hidden="true" />
              </button>
            </header>
            <pre>{content.value}</pre>
          </section>
        )}
      </div>
    </aside>
  );
}

export function SessionActivityView({
  sourceRunId,
  available,
  loadPage,
}: SessionActivityViewProps) {
  const [activities, setActivities] = useState<SessionActivity[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState<ActivityFilters>(EMPTY_FILTERS);
  const [filterOpen, setFilterOpen] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [nextPageToken, setNextPageToken] = useState("");
  const [caughtUp, setCaughtUp] = useState(true);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState("");
  const [live, setLive] = useState(true);
  const [zoom, setZoom] = useState(1);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const requestGeneration = useRef(0);
  const loadPageRef = useRef(loadPage);
  const [debouncedQuery, setDebouncedQuery] = useState(query);
  const groups = useMemo(() => activityGroups(activities), [activities]);
  const selected =
    activities.find((activity) => activity.activityId === selectedId) ?? null;
  const filterCount =
    filters.lanes.length + filters.kinds.length + filters.statuses.length;

  useEffect(() => {
    loadPageRef.current = loadPage;
  }, [loadPage]);

  useEffect(() => {
    const timeout = window.setTimeout(
      () => setDebouncedQuery(query),
      SEARCH_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [query]);

  const fetchFirstPage = useCallback(
    async (merge: boolean) => {
      if (!available) {
        return;
      }
      const generation = merge
        ? requestGeneration.current
        : ++requestGeneration.current;
      if (!merge) {
        setLoading(true);
        setError("");
      }
      try {
        const page = await loadPageRef.current(
          activityRequest(sourceRunId, debouncedQuery, filters),
        );
        if (generation !== requestGeneration.current) {
          return;
        }
        setActivities((current) =>
          merge ? mergeActivities(current, page.activities) : page.activities,
        );
        setNextPageToken(page.nextPageToken);
        setCaughtUp(page.caughtUp);
        setError("");
      } catch (caught: unknown) {
        if (generation === requestGeneration.current) {
          setError(
            caught instanceof Error
              ? caught.message
              : "Session activity is unavailable.",
          );
        }
      } finally {
        if (!merge && generation === requestGeneration.current) {
          setLoading(false);
        }
      }
    },
    [available, debouncedQuery, filters, sourceRunId],
  );

  useEffect(() => {
    setActivities([]);
    setSelectedId(null);
    setExpanded(new Set());
    void fetchFirstPage(false);
  }, [fetchFirstPage]);

  useEffect(() => {
    if (!live || !available) {
      return undefined;
    }
    const interval = window.setInterval(() => {
      void fetchFirstPage(true);
    }, LIVE_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [available, fetchFirstPage, live]);

  useEffect(() => {
    const first = groups[0];
    if (first === undefined) {
      return;
    }
    setExpanded((current) => {
      if (current.size > 0) {
        return current;
      }
      return new Set([first.key]);
    });
    const initialActivity =
      first.activities.find((activity) => activity.kind === "tool") ??
      first.activities.at(-1);
    setSelectedId((current) => current ?? initialActivity?.activityId ?? null);
  }, [groups]);

  const selectActivity = (activity: SessionActivity) => {
    setSelectedId(activity.activityId);
    const group = groups.find((candidate) =>
      candidate.activities.some(
        (value) => value.activityId === activity.activityId,
      ),
    );
    if (group !== undefined) {
      setExpanded((current) => new Set(current).add(group.key));
      window.requestAnimationFrame(() =>
        rowRefs.current
          .get(activity.activityId)
          ?.scrollIntoView({ block: "nearest" }),
      );
    }
  };

  const moveSelection = (
    event: KeyboardEvent<HTMLButtonElement>,
    id: string,
  ) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") {
      return;
    }
    const rows = groups.flatMap((group) =>
      expanded.has(group.key)
        ? group.activities.map((activity) => activity.activityId)
        : [],
    );
    const index = rows.indexOf(id);
    const next = rows[index + (event.key === "ArrowDown" ? 1 : -1)];
    if (next !== undefined) {
      event.preventDefault();
      rowRefs.current.get(next)?.focus();
      setSelectedId(next);
    }
  };

  const loadMore = async () => {
    if (nextPageToken === "" || loadingMore) {
      return;
    }
    setLoadingMore(true);
    try {
      const page = await loadPageRef.current(
        activityRequest(sourceRunId, debouncedQuery, filters, nextPageToken),
      );
      setActivities((current) => mergeActivities(current, page.activities));
      setNextPageToken(page.nextPageToken);
      setCaughtUp(page.caughtUp);
    } catch (caught: unknown) {
      setError(
        caught instanceof Error
          ? caught.message
          : "More activity could not be loaded.",
      );
    } finally {
      setLoadingMore(false);
    }
  };

  if (!available) {
    return (
      <section
        className="session-activity-state"
        aria-labelledby="session-activity-title"
      >
        <IconAlertTriangle size={24} stroke={1.5} aria-hidden="true" />
        <h3 id="session-activity-title">
          Activity requires a newer Colossus target
        </h3>
        <p>
          Upgrade the selected runtime to one that advertises sessions.activity.
        </p>
      </section>
    );
  }

  return (
    <section
      className="session-activity"
      aria-labelledby="session-activity-title"
    >
      <header className="session-activity-toolbar">
        <div>
          <p className="eyebrow">Canonical journal</p>
          <h3 id="session-activity-title">Session activity</h3>
        </div>
        <div className="session-activity-actions">
          <label className="activity-search">
            <IconSearch size={17} stroke={1.7} aria-hidden="true" />
            <span className="sr-only">Search session activity</span>
            <input
              type="search"
              value={query}
              placeholder="Search activity…"
              onChange={(event) => setQuery(event.currentTarget.value)}
            />
          </label>
          <div className="activity-filter-wrap">
            <button
              type="button"
              className="activity-toolbar-button"
              aria-expanded={filterOpen}
              aria-haspopup="dialog"
              onClick={() => setFilterOpen((current) => !current)}
            >
              <IconFilter size={16} stroke={1.7} aria-hidden="true" />
              Filter{filterCount > 0 ? ` (${filterCount})` : ""}
            </button>
            {filterOpen ? (
              <div
                className="activity-filter-popover"
                role="dialog"
                aria-label="Activity filters"
              >
                <fieldset>
                  <legend>Lane</legend>
                  {LANES.map(({ lane, label }) => (
                    <label key={lane}>
                      <input
                        type="checkbox"
                        checked={filters.lanes.includes(lane)}
                        onChange={() =>
                          setFilters((current) => ({
                            ...current,
                            lanes: toggleFilter(current.lanes, lane),
                          }))
                        }
                      />
                      {label}
                    </label>
                  ))}
                </fieldset>
                <fieldset>
                  <legend>Type</legend>
                  {KIND_OPTIONS.map((option) => (
                    <label key={option.value}>
                      <input
                        type="checkbox"
                        checked={filters.kinds.includes(option.value)}
                        onChange={() =>
                          setFilters((current) => ({
                            ...current,
                            kinds: toggleFilter(current.kinds, option.value),
                          }))
                        }
                      />
                      {option.label}
                    </label>
                  ))}
                </fieldset>
                <fieldset>
                  <legend>Status</legend>
                  {STATUS_OPTIONS.map((option) => (
                    <label key={option.value}>
                      <input
                        type="checkbox"
                        checked={filters.statuses.includes(option.value)}
                        onChange={() =>
                          setFilters((current) => ({
                            ...current,
                            statuses: toggleFilter(
                              current.statuses,
                              option.value,
                            ),
                          }))
                        }
                      />
                      {option.label}
                    </label>
                  ))}
                </fieldset>
                <button type="button" onClick={() => setFilters(EMPTY_FILTERS)}>
                  Clear filters
                </button>
              </div>
            ) : null}
          </div>
          <button
            type="button"
            className="activity-toolbar-button"
            aria-pressed={live}
            onClick={() => setLive((current) => !current)}
          >
            {live ? (
              <IconPlayerPause size={16} stroke={1.7} aria-hidden="true" />
            ) : (
              <IconPlayerPlay size={16} stroke={1.7} aria-hidden="true" />
            )}
            {live ? "Live" : "Paused"}
          </button>
        </div>
      </header>

      {!caughtUp ? (
        <div className="activity-catching-up" role="status">
          <IconRefresh size={15} stroke={1.7} aria-hidden="true" />
          Catching up to the canonical journal…
        </div>
      ) : null}

      <div className="activity-timeline-shell">
        <ActivityTimeline
          activities={activities}
          selectedId={selectedId}
          zoom={zoom}
          onSelect={selectActivity}
        />
        <div className="activity-zoom-controls" aria-label="Timeline zoom">
          <button
            type="button"
            aria-label="Zoom out"
            disabled={zoom <= 1}
            onClick={() => setZoom((current) => Math.max(1, current - 0.5))}
          >
            <IconZoomOut size={16} stroke={1.7} aria-hidden="true" />
          </button>
          <button type="button" onClick={() => setZoom(1)}>
            {zoom.toFixed(1)}×
          </button>
          <button
            type="button"
            aria-label="Zoom in"
            disabled={zoom >= 4}
            onClick={() => setZoom((current) => Math.min(4, current + 0.5))}
          >
            <IconZoomIn size={16} stroke={1.7} aria-hidden="true" />
          </button>
        </div>
      </div>

      {loading ? (
        <div className="session-activity-state" role="status">
          <span className="activity-loading-dot" />
          <h3>Loading session activity</h3>
          <p>Reading the curated session projection.</p>
        </div>
      ) : error !== "" && activities.length === 0 ? (
        <div className="session-activity-state" role="alert">
          <IconAlertTriangle size={24} stroke={1.5} aria-hidden="true" />
          <h3>Session activity could not be loaded</h3>
          <p>{error}</p>
          <button type="button" onClick={() => void fetchFirstPage(false)}>
            <IconRefresh size={16} stroke={1.7} aria-hidden="true" />
            Retry
          </button>
        </div>
      ) : activities.length === 0 ? (
        <div className="session-activity-state">
          <IconSearch size={24} stroke={1.5} aria-hidden="true" />
          <h3>
            {query || filterCount > 0
              ? "No matching activity"
              : "No activity yet"}
          </h3>
          <p>
            {query || filterCount > 0
              ? "Adjust the search or filters to see more events."
              : "Released session events will appear here as the run progresses."}
          </p>
        </div>
      ) : (
        <div className="activity-split-pane">
          <section className="activity-feed" aria-label="Session activity feed">
            <div className="activity-feed-heading" aria-hidden="true">
              <span>Time</span>
              <span>Type</span>
              <span>Event</span>
              <span>Actor</span>
              <span>Duration</span>
            </div>
            {groups.map((group) => {
              const isExpanded = expanded.has(group.key);
              return (
                <div className="activity-group" key={group.key}>
                  <button
                    type="button"
                    className="activity-group-heading"
                    aria-expanded={isExpanded}
                    onClick={() =>
                      setExpanded((current) => {
                        const next = new Set(current);
                        if (next.has(group.key)) next.delete(group.key);
                        else next.add(group.key);
                        return next;
                      })
                    }
                  >
                    {isExpanded ? (
                      <IconChevronDown
                        size={16}
                        stroke={1.8}
                        aria-hidden="true"
                      />
                    ) : (
                      <IconChevronRight
                        size={16}
                        stroke={1.8}
                        aria-hidden="true"
                      />
                    )}
                    <strong>{groupLabel(group)}</strong>
                    <span>{shortTime(group.startedAt)}</span>
                    <span>{group.activities.length} events</span>
                  </button>
                  {isExpanded
                    ? group.activities.map((activity) => (
                        <button
                          ref={(node) => {
                            if (node === null)
                              rowRefs.current.delete(activity.activityId);
                            else rowRefs.current.set(activity.activityId, node);
                          }}
                          type="button"
                          className="activity-row"
                          data-selected={selectedId === activity.activityId}
                          key={activity.activityId}
                          onClick={() => selectActivity(activity)}
                          onKeyDown={(event) =>
                            moveSelection(event, activity.activityId)
                          }
                        >
                          <span>{shortTime(activity.startedAt)}</span>
                          <span
                            className={`activity-kind kind-${activity.kind}`}
                          >
                            {activityIcon(activity)}
                            {activity.kind}
                          </span>
                          <span>
                            <strong>{activity.title}</strong>
                            <small>{activity.summary}</small>
                          </span>
                          <span>{activity.actor}</span>
                          <span>{durationLabel(activity.durationMs)}</span>
                        </button>
                      ))
                    : null}
                </div>
              );
            })}
            {error !== "" ? (
              <p className="activity-inline-error">{error}</p>
            ) : null}
            {nextPageToken !== "" ? (
              <button
                className="activity-load-more"
                type="button"
                disabled={loadingMore}
                onClick={() => void loadMore()}
              >
                {loadingMore ? "Loading…" : "Load earlier activity"}
              </button>
            ) : (
              <footer>
                <IconCheck size={14} stroke={1.8} aria-hidden="true" />
                {activities.length} curated events
              </footer>
            )}
          </section>
          <ActivityInspector activity={selected} />
        </div>
      )}
    </section>
  );
}

export { activityGroups, mergeActivities };
