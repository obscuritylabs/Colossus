import {
  IconAlertTriangle,
  IconArrowsHorizontal,
  IconBroadcast,
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconClock,
  IconCopy,
  IconFilter,
  IconFocus2,
  IconHandGrab,
  IconPlayerPause,
  IconPlayerPlay,
  IconRefresh,
  IconRobot,
  IconSearch,
  IconSettings,
  IconTool,
  IconUser,
  IconX,
  IconZoomIn,
  IconZoomOut,
  IconZoomScan,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  KeyboardEvent,
  PointerEvent as ReactPointerEvent,
  WheelEvent as ReactWheelEvent,
} from "react";

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
const MAX_TIMELINE_ACTIVITIES = 500;
const DEFAULT_TIMELINE_SPAN_MS = 60_000;
const MIN_TIMELINE_SPAN_MS = 1_000;
const TIMELINE_PAN_STEP = 0.1;

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
  runRole: string;
  subagentRole: string | null;
  parentRunId: string | null;
  activities: SessionActivity[];
  newestSequence: number;
  startedAt: string;
}

export interface TimelineRange {
  start: number;
  end: number;
}

type TimelineNavigationMode = "pan" | "range";

interface TimelineNavigationState {
  range: TimelineRange | null;
  follow: boolean;
}

type TimelineDrag =
  | {
      kind: "pan";
      pointerId: number;
      startX: number;
      startRange: TimelineRange;
    }
  | {
      kind: "range";
      pointerId: number;
      startX: number;
      startTime: number;
    };

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
  { value: "requested", label: "Requested" },
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

function historyTokenAfterHeadRefresh(
  current: string,
  incoming: string,
  merge: boolean,
): string {
  return merge ? current : incoming;
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
      runRole: activity.attributes.run_role ?? "primary",
      subagentRole: activity.attributes.subagent_role ?? null,
      parentRunId: activity.attributes.parent_run_id ?? null,
      activities: [],
      newestSequence: activity.lastSequence,
      startedAt: activity.startedAt,
    };
    group.activities.push(activity);
    if (activity.attributes.run_role !== undefined) {
      group.runRole = activity.attributes.run_role;
    }
    if (activity.attributes.subagent_role !== undefined) {
      group.subagentRole = activity.attributes.subagent_role;
    }
    if (activity.attributes.parent_run_id !== undefined) {
      group.parentRunId = activity.attributes.parent_run_id;
    }
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
  const turn = group.turn === null ? "Run activity" : `Turn ${group.turn}`;
  if (group.runRole === "subagent") {
    return group.subagentRole === null
      ? `Subagent · ${turn}`
      : `Subagent · ${group.subagentRole} · ${turn}`;
  }
  if (group.runRole === "workflow") {
    return `Workflow · ${turn}`;
  }
  return group.runId === null ? "Session" : `Primary · ${turn}`;
}

function shortId(value: string): string {
  return value.length <= 12 ? value : `${value.slice(0, 8)}…`;
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

function rangeSpan(range: TimelineRange): number {
  return Math.max(range.end - range.start, MIN_TIMELINE_SPAN_MS);
}

export function timelineExtent(
  activities: readonly SessionActivity[],
  fallbackNow = Date.now(),
): TimelineRange {
  let earliest = Number.POSITIVE_INFINITY;
  let latest = Number.NEGATIVE_INFINITY;
  for (const activity of activities) {
    const started = Date.parse(activity.startedAt);
    const completed =
      activity.completedAt === null
        ? Number.NaN
        : Date.parse(activity.completedAt);
    if (Number.isFinite(started)) {
      earliest = Math.min(earliest, started);
      latest = Math.max(latest, started);
    }
    if (Number.isFinite(completed)) {
      earliest = Math.min(earliest, completed);
      latest = Math.max(latest, completed);
    }
  }
  if (!Number.isFinite(earliest) || !Number.isFinite(latest)) {
    latest = fallbackNow;
    earliest = latest - DEFAULT_TIMELINE_SPAN_MS;
  }
  return {
    start: Math.min(earliest, latest - DEFAULT_TIMELINE_SPAN_MS),
    end: latest,
  };
}

export function clampTimelineRange(
  range: TimelineRange,
  extent: TimelineRange,
): TimelineRange {
  const extentSpan = Math.max(extent.end - extent.start, MIN_TIMELINE_SPAN_MS);
  const span = Math.min(
    Math.max(range.end - range.start, MIN_TIMELINE_SPAN_MS),
    extentSpan,
  );
  let start = range.start;
  if (start < extent.start) {
    start = extent.start;
  }
  if (start + span > extent.end) {
    start = extent.end - span;
  }
  return { start, end: start + span };
}

export function panTimelineRange(
  range: TimelineRange,
  extent: TimelineRange,
  deltaMs: number,
): TimelineRange {
  return clampTimelineRange(
    { start: range.start + deltaMs, end: range.end + deltaMs },
    extent,
  );
}

export function zoomTimelineRange(
  range: TimelineRange,
  extent: TimelineRange,
  scale: number,
  anchor = (range.start + range.end) / 2,
): TimelineRange {
  const currentSpan = rangeSpan(range);
  const extentSpan = rangeSpan(extent);
  const nextSpan = Math.min(
    Math.max(currentSpan * scale, MIN_TIMELINE_SPAN_MS),
    extentSpan,
  );
  const anchorRatio = Math.min(
    1,
    Math.max(0, (anchor - range.start) / currentSpan),
  );
  const start = anchor - nextSpan * anchorRatio;
  return clampTimelineRange({ start, end: start + nextSpan }, extent);
}

function normalizeTimelineRange(left: number, right: number): TimelineRange {
  return left <= right
    ? { start: left, end: right }
    : { start: right, end: left };
}

function timelineZoomLabel(
  extent: TimelineRange,
  range: TimelineRange,
): string {
  const zoom = rangeSpan(extent) / rangeSpan(range);
  return `${zoom < 10 ? zoom.toFixed(1) : Math.round(zoom)}×`;
}

function timelineRangeLabel(range: TimelineRange): string {
  return `${shortTime(new Date(range.start).toISOString())} – ${shortTime(
    new Date(range.end).toISOString(),
  )} · ${durationLabel(range.end - range.start)}`;
}

function ActivityTimeline({
  activities,
  selectedId,
  live,
  onSelect,
}: {
  activities: readonly SessionActivity[];
  selectedId: string | null;
  live: boolean;
  onSelect: (activity: SessionActivity) => void;
}) {
  const extent = useMemo(() => timelineExtent(activities), [activities]);
  const [navigation, setNavigation] = useState<TimelineNavigationState>({
    range: null,
    follow: true,
  });
  const [mode, setMode] = useState<TimelineNavigationMode>("pan");
  const [selection, setSelection] = useState<TimelineRange | null>(null);
  const [dragSelection, setDragSelection] = useState<TimelineRange | null>(
    null,
  );
  const [dragging, setDragging] = useState<TimelineDrag["kind"] | "none">(
    "none",
  );
  const firstTrackRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<TimelineDrag | null>(null);
  const initializedWithActivity = useRef(false);

  useEffect(() => {
    if (activities.length === 0 || initializedWithActivity.current) {
      return;
    }
    initializedWithActivity.current = true;
    setNavigation({ range: extent, follow: true });
  }, [activities.length, extent]);

  const viewRange = useMemo(() => {
    const requested = navigation.range ?? extent;
    if (!navigation.follow) {
      return clampTimelineRange(requested, extent);
    }
    const span = Math.min(rangeSpan(requested), rangeSpan(extent));
    return clampTimelineRange(
      { start: extent.end - span, end: extent.end },
      extent,
    );
  }, [extent, navigation]);
  const viewSpan = rangeSpan(viewRange);
  const visibleSelection = dragSelection ?? selection;
  const selectionStyle = useMemo(() => {
    if (
      visibleSelection === null ||
      visibleSelection.end < viewRange.start ||
      visibleSelection.start > viewRange.end
    ) {
      return null;
    }
    const start = Math.max(visibleSelection.start, viewRange.start);
    const end = Math.min(visibleSelection.end, viewRange.end);
    return {
      left: `${((start - viewRange.start) / viewSpan) * 100}%`,
      width: `${Math.max(((end - start) / viewSpan) * 100, 0.2)}%`,
    };
  }, [viewRange, viewSpan, visibleSelection]);
  const ticks = Array.from({ length: 7 }, (_, index) => {
    const time = viewRange.start + (viewSpan * index) / 6;
    return {
      left: `${(index / 6) * 100}%`,
      label: shortTime(new Date(time).toISOString()),
    };
  });

  const manuallySetRange = (range: TimelineRange) => {
    setNavigation({ range: clampTimelineRange(range, extent), follow: false });
  };

  const pointerTime = (clientX: number): number | null => {
    const bounds = firstTrackRef.current?.getBoundingClientRect();
    if (bounds === undefined || bounds.width <= 0) {
      return null;
    }
    const fraction = Math.min(
      1,
      Math.max(0, (clientX - bounds.left) / bounds.width),
    );
    return viewRange.start + fraction * viewSpan;
  };

  const handlePointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (
      event.button !== 0 ||
      !(event.target instanceof Element) ||
      event.target.closest("button") !== null
    ) {
      return;
    }
    const trackBounds = firstTrackRef.current?.getBoundingClientRect();
    if (
      trackBounds === undefined ||
      event.clientX < trackBounds.left ||
      event.clientX > trackBounds.right
    ) {
      return;
    }
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    setNavigation({ range: viewRange, follow: false });
    if (mode === "pan") {
      setDragging("pan");
      dragRef.current = {
        kind: "pan",
        pointerId: event.pointerId,
        startX: event.clientX,
        startRange: viewRange,
      };
      return;
    }
    const startTime = pointerTime(event.clientX);
    if (startTime === null) {
      return;
    }
    dragRef.current = {
      kind: "range",
      pointerId: event.pointerId,
      startX: event.clientX,
      startTime,
    };
    setDragging("range");
    setSelection(null);
    setDragSelection({ start: startTime, end: startTime });
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    if (drag === null || drag.pointerId !== event.pointerId) {
      return;
    }
    const trackBounds = firstTrackRef.current?.getBoundingClientRect();
    if (trackBounds === undefined || trackBounds.width <= 0) {
      return;
    }
    event.preventDefault();
    if (drag.kind === "pan") {
      const deltaMs =
        (-(event.clientX - drag.startX) / trackBounds.width) *
        rangeSpan(drag.startRange);
      manuallySetRange(panTimelineRange(drag.startRange, extent, deltaMs));
      return;
    }
    const currentTime = pointerTime(event.clientX);
    if (currentTime !== null) {
      setDragSelection(normalizeTimelineRange(drag.startTime, currentTime));
    }
  };

  const finishPointerGesture = (event: ReactPointerEvent<HTMLElement>) => {
    const drag = dragRef.current;
    if (drag === null || drag.pointerId !== event.pointerId) {
      return;
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    if (drag.kind === "range") {
      const currentTime = pointerTime(event.clientX);
      const moved = Math.abs(event.clientX - drag.startX) >= 4;
      setSelection(
        moved && currentTime !== null
          ? normalizeTimelineRange(drag.startTime, currentTime)
          : null,
      );
      setDragSelection(null);
    }
    dragRef.current = null;
    setDragging("none");
  };

  const cancelPointerGesture = (event: ReactPointerEvent<HTMLElement>) => {
    if (dragRef.current?.pointerId !== event.pointerId) {
      return;
    }
    dragRef.current = null;
    setDragging("none");
    setDragSelection(null);
  };

  const zoomBy = (scale: number, anchor?: number) => {
    manuallySetRange(zoomTimelineRange(viewRange, extent, scale, anchor));
  };

  const panBy = (fraction: number) => {
    manuallySetRange(panTimelineRange(viewRange, extent, viewSpan * fraction));
  };

  const handleWheel = (event: ReactWheelEvent<HTMLElement>) => {
    if (event.ctrlKey || event.metaKey) {
      event.preventDefault();
      const anchor = pointerTime(event.clientX) ?? undefined;
      zoomBy(event.deltaY < 0 ? 0.8 : 1.25, anchor);
      return;
    }
    const horizontalDelta =
      Math.abs(event.deltaX) > Math.abs(event.deltaY)
        ? event.deltaX
        : event.shiftKey
          ? event.deltaY
          : 0;
    if (horizontalDelta !== 0) {
      event.preventDefault();
      panBy(horizontalDelta / 500);
    }
  };

  const handleKeyboardNavigation = (event: KeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget) {
      return;
    }
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      panBy(event.key === "ArrowLeft" ? -TIMELINE_PAN_STEP : TIMELINE_PAN_STEP);
    } else if (event.key === "+" || event.key === "=") {
      event.preventDefault();
      zoomBy(0.8);
    } else if (event.key === "-" || event.key === "_") {
      event.preventDefault();
      zoomBy(1.25);
    } else if (event.key === "Home") {
      event.preventDefault();
      setNavigation({ range: extent, follow: false });
    } else if (event.key === "End") {
      event.preventDefault();
      setNavigation({ range: viewRange, follow: true });
    } else if (event.key === "Escape") {
      setSelection(null);
      setDragSelection(null);
    }
  };

  const zoomLabel = timelineZoomLabel(extent, viewRange);
  const atFullExtent = viewSpan >= rangeSpan(extent) - 1;
  const atMinimumSpan = viewSpan <= MIN_TIMELINE_SPAN_MS + 1;
  return (
    <section
      className="activity-timeline"
      aria-label="Session activity timeline"
      aria-describedby="activity-timeline-help"
      data-mode={mode}
      data-dragging={dragging}
      onKeyDown={handleKeyboardNavigation}
      onPointerCancel={cancelPointerGesture}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={finishPointerGesture}
      onWheel={handleWheel}
      tabIndex={0}
    >
      <span className="sr-only" id="activity-timeline-help">
        Use Pan mode to drag through time, or Range mode to drag-select an
        interval. Arrow keys pan, plus and minus zoom, Home fits all activity,
        End follows live activity, and Escape clears the selected range.
      </span>
      {LANES.map(({ lane, label }, laneIndex) => (
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
          <div
            className="activity-timeline-track"
            data-timeline-track=""
            ref={laneIndex === 0 ? firstTrackRef : undefined}
          >
            {selectionStyle === null ? null : (
              <span
                className="activity-timeline-selection"
                aria-hidden="true"
                style={selectionStyle}
              />
            )}
            {activities
              .filter((activity) => activity.lane === lane)
              .map((activity) => {
                const started = Date.parse(activity.startedAt);
                if (!Number.isFinite(started)) {
                  return null;
                }
                const parsedCompleted = activity.completedAt
                  ? Date.parse(activity.completedAt)
                  : started;
                const completed = Number.isFinite(parsedCompleted)
                  ? Math.max(parsedCompleted, started)
                  : started;
                if (started > viewRange.end || completed < viewRange.start) {
                  return null;
                }
                const visibleStart = Math.max(started, viewRange.start);
                const visibleEnd = Math.min(completed, viewRange.end);
                const left =
                  ((visibleStart - viewRange.start) / viewSpan) * 100;
                const width = Math.max(
                  ((visibleEnd - visibleStart) / viewSpan) * 100,
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
      <div className="activity-timeline-navigator">
        <div
          className="activity-navigation-modes"
          role="group"
          aria-label="Timeline interaction mode"
        >
          <button
            type="button"
            aria-label="Pan timeline"
            aria-pressed={mode === "pan"}
            title="Pan timeline (drag)"
            onClick={() => setMode("pan")}
          >
            <IconHandGrab size={15} stroke={1.7} aria-hidden="true" />
            Pan
          </button>
          <button
            type="button"
            aria-label="Select time range"
            aria-pressed={mode === "range"}
            title="Select a time range (drag)"
            onClick={() => setMode("range")}
          >
            <IconArrowsHorizontal size={15} stroke={1.7} aria-hidden="true" />
            Range
          </button>
        </div>
        <div className="activity-range-readout" role="status">
          {selection === null ? (
            <span>
              {navigation.follow && live ? "Following newest activity" : ""}
            </span>
          ) : (
            <>
              <span>{timelineRangeLabel(selection)}</span>
              <button
                type="button"
                aria-label="Clear selected time range"
                onClick={() => setSelection(null)}
              >
                <IconX size={14} stroke={1.8} aria-hidden="true" />
              </button>
            </>
          )}
        </div>
        <div
          className="activity-zoom-controls"
          aria-label="Timeline navigation"
        >
          <button
            type="button"
            aria-label="Zoom out"
            disabled={atFullExtent}
            onClick={() => zoomBy(1.6)}
          >
            <IconZoomOut size={16} stroke={1.7} aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label="Fit entire timeline"
            title="Fit all activity (Home)"
            onClick={() => setNavigation({ range: extent, follow: false })}
          >
            <IconFocus2 size={15} stroke={1.7} aria-hidden="true" />
            {zoomLabel}
          </button>
          <button
            type="button"
            aria-label="Zoom in"
            disabled={atMinimumSpan}
            onClick={() => zoomBy(0.625)}
          >
            <IconZoomIn size={16} stroke={1.7} aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label="Zoom to selected time range"
            disabled={selection === null}
            title="Zoom to selected range"
            onClick={() => {
              if (selection !== null) {
                manuallySetRange(selection);
              }
            }}
          >
            <IconZoomScan size={16} stroke={1.7} aria-hidden="true" />
            Range
          </button>
          <button
            type="button"
            className="activity-follow-live"
            aria-label="Follow live activity"
            aria-pressed={navigation.follow}
            data-live={live}
            title="Keep the current time span pinned to the newest activity (End)"
            onClick={() =>
              setNavigation({ range: viewRange, follow: !navigation.follow })
            }
          >
            <IconBroadcast size={16} stroke={1.7} aria-hidden="true" />
            Follow
          </button>
        </div>
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
                <dt>Run</dt>
                <dd title={activity.runId ?? undefined}>
                  {activity.runId === null ? "—" : shortId(activity.runId)}
                </dd>
              </div>
              {activity.attributes.parent_run_id === undefined ? null : (
                <div>
                  <dt>Parent run</dt>
                  <dd title={activity.attributes.parent_run_id}>
                    {shortId(activity.attributes.parent_run_id)}
                  </dd>
                </div>
              )}
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
  const [historyPageToken, setHistoryPageToken] = useState("");
  const [caughtUp, setCaughtUp] = useState(true);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [initialError, setInitialError] = useState("");
  const [refreshError, setRefreshError] = useState("");
  const [historyError, setHistoryError] = useState("");
  const [live, setLive] = useState(true);
  const rowRefs = useRef(new Map<string, HTMLButtonElement>());
  const requestGeneration = useRef(0);
  const liveRequestInFlight = useRef(false);
  const loadPageRef = useRef(loadPage);
  const [debouncedQuery, setDebouncedQuery] = useState(query);
  const groups = useMemo(() => activityGroups(activities), [activities]);
  const selected =
    activities.find((activity) => activity.activityId === selectedId) ?? null;
  const timelineActivities = useMemo(() => {
    if (activities.length <= MAX_TIMELINE_ACTIVITIES) {
      return activities;
    }
    const latest = activities.slice(0, MAX_TIMELINE_ACTIVITIES);
    if (
      selected === null ||
      latest.some((activity) => activity.activityId === selected.activityId)
    ) {
      return latest;
    }
    return [...latest.slice(0, -1), selected];
  }, [activities, selected]);
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
      if (merge && liveRequestInFlight.current) {
        return;
      }
      const generation = merge
        ? requestGeneration.current
        : ++requestGeneration.current;
      if (merge) {
        liveRequestInFlight.current = true;
      } else {
        setLoading(true);
        setInitialError("");
        setRefreshError("");
        setHistoryError("");
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
        setHistoryPageToken((current) =>
          historyTokenAfterHeadRefresh(current, page.nextPageToken, merge),
        );
        setCaughtUp(page.caughtUp);
        if (merge) {
          setRefreshError("");
        } else {
          setInitialError("");
        }
      } catch (caught: unknown) {
        if (generation === requestGeneration.current) {
          const message =
            caught instanceof Error
              ? caught.message
              : "Session activity is unavailable.";
          if (merge) {
            setRefreshError(message);
          } else {
            setInitialError(message);
          }
        }
      } finally {
        if (merge) {
          liveRequestInFlight.current = false;
        } else if (generation === requestGeneration.current) {
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
    setHistoryPageToken("");
    setLoadingMore(false);
    setHistoryError("");
    liveRequestInFlight.current = false;
    void fetchFirstPage(false);
    return () => {
      requestGeneration.current += 1;
      liveRequestInFlight.current = false;
    };
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
    if (historyPageToken === "" || loadingMore) {
      return;
    }
    const generation = requestGeneration.current;
    setLoadingMore(true);
    setHistoryError("");
    try {
      const page = await loadPageRef.current(
        activityRequest(sourceRunId, debouncedQuery, filters, historyPageToken),
      );
      if (generation !== requestGeneration.current) {
        return;
      }
      setActivities((current) => mergeActivities(current, page.activities));
      setHistoryPageToken(page.nextPageToken);
      setCaughtUp(page.caughtUp);
    } catch (caught: unknown) {
      if (generation === requestGeneration.current) {
        setHistoryError(
          caught instanceof Error
            ? caught.message
            : "More activity could not be loaded.",
        );
      }
    } finally {
      if (generation === requestGeneration.current) {
        setLoadingMore(false);
      }
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
      {refreshError !== "" ? (
        <div className="activity-degraded-live" role="alert">
          <IconAlertTriangle size={15} stroke={1.7} aria-hidden="true" />
          <span>
            <strong>Live refresh failed.</strong> Showing the last successful
            activity snapshot. {refreshError}
          </span>
          <button type="button" onClick={() => void fetchFirstPage(true)}>
            Retry now
          </button>
        </div>
      ) : null}

      <div className="activity-timeline-shell">
        <ActivityTimeline
          activities={timelineActivities}
          key={sourceRunId}
          live={live}
          selectedId={selectedId}
          onSelect={selectActivity}
        />
      </div>

      {loading ? (
        <div className="session-activity-state" role="status">
          <span className="activity-loading-dot" />
          <h3>Loading session activity</h3>
          <p>Reading the curated session projection.</p>
        </div>
      ) : initialError !== "" && activities.length === 0 ? (
        <div className="session-activity-state" role="alert">
          <IconAlertTriangle size={24} stroke={1.5} aria-hidden="true" />
          <h3>Session activity could not be loaded</h3>
          <p>{initialError}</p>
          <button type="button" onClick={() => void fetchFirstPage(false)}>
            <IconRefresh size={16} stroke={1.7} aria-hidden="true" />
            Retry
          </button>
        </div>
      ) : activities.length === 0 ? (
        <div className="session-activity-state">
          <IconSearch size={24} stroke={1.5} aria-hidden="true" />
          <h3>
            {historyPageToken !== ""
              ? "No matches in the latest activity"
              : query || filterCount > 0
                ? "No matching activity"
                : "No activity yet"}
          </h3>
          <p>
            {historyPageToken !== ""
              ? "Continue into earlier canonical history to look for matching activity."
              : query || filterCount > 0
                ? "Adjust the search or filters to see more events."
                : "Released session events will appear here as the run progresses."}
          </p>
          {historyError !== "" ? (
            <p className="activity-inline-error" role="alert">
              {historyError}
            </p>
          ) : null}
          {historyPageToken !== "" ? (
            <button
              type="button"
              disabled={loadingMore}
              onClick={() => void loadMore()}
            >
              {loadingMore ? "Searching…" : "Search earlier activity"}
            </button>
          ) : null}
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
                    <span className="activity-group-title">
                      <strong>{groupLabel(group)}</strong>
                      {group.parentRunId === null ? null : (
                        <small>Parent run {shortId(group.parentRunId)}</small>
                      )}
                    </span>
                    <span>{shortTime(group.startedAt)}</span>
                    <span>
                      {group.activities.length}{" "}
                      {group.activities.length === 1 ? "event" : "events"}
                    </span>
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
            {historyError !== "" ? (
              <div className="activity-history-error" role="alert">
                <span>{historyError}</span>
                <button type="button" onClick={() => void loadMore()}>
                  Retry loading earlier activity
                </button>
              </div>
            ) : null}
            {historyPageToken !== "" ? (
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

export { activityGroups, historyTokenAfterHeadRefresh, mergeActivities };
