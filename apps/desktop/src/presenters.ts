import type { RunView } from "./state";
import type {
  ArtifactReference,
  Run,
  RunMode,
  RunStatus,
  RunUpdate,
  ToolActivityState,
} from "./types";

export const MAX_PRESENTED_WORK_ITEMS = 200;
export const MAX_PRESENTED_ARTIFACTS = 64;

const MAX_PRESENTED_VIEWS = 12;
const MAX_PRESENTED_UPDATES_PER_VIEW = 200;
const MAX_LABEL_CHARACTERS = 72;
const MAX_DETAIL_CHARACTERS = 280;
const MAX_FILE_NAME_CHARACTERS = 120;
const ACTIVE_STATUSES: ReadonlySet<RunStatus> = new Set([
  "queued",
  "running",
  "waiting",
  "cancelling",
]);

export type PresentationTone =
  "neutral" | "progress" | "attention" | "success" | "danger";

export interface StatusPresentation {
  label: string;
  copy: string;
  tone: PresentationTone;
}

export type WorkGroupKey = "pinned" | "attention" | "active" | "recent";

export interface RecentWorkItem {
  /** Opaque routing value. It is never used as display copy. */
  runId: string;
  title: string;
  mode: RunMode;
  modeLabel: string;
  status: RunStatus;
  statusLabel: string;
  statusCopy: string;
  statusTone: PresentationTone;
  updatedAt: string;
  updatedLabel: string;
  isActive: boolean;
  needsAttention: boolean;
}

export interface RecentWorkGroup {
  key: WorkGroupKey;
  label: string;
  items: RecentWorkItem[];
}

export interface RecentWorkOptions {
  query?: string;
  now?: Date;
  limit?: number;
  pinnedSessionIds?: ReadonlySet<string>;
}

export interface PresentedArtifact {
  /** Opaque routing value for a future authorized artifact action. */
  artifactId: string;
  key: string;
  fileName: string;
  mediaType: string;
  typeLabel: string;
  sizeBytes: number;
  sizeLabel: string;
  purpose: ArtifactReference["purpose"];
  purposeLabel: string;
  state: ArtifactReference["state"];
  stateLabel: string;
  canOpen: boolean;
  createdAt: string;
  createdLabel: string;
  /** Opaque routing value. It is never used as display copy. */
  runId: string;
}

type RunViewSource = RunView | Iterable<RunView> | null | undefined;

const STATUS_PRESENTATIONS: Readonly<Record<RunStatus, StatusPresentation>> = {
  queued: {
    label: "Queued",
    copy: "Waiting to start",
    tone: "neutral",
  },
  running: {
    label: "In progress",
    copy: "Work is in progress",
    tone: "progress",
  },
  waiting: {
    label: "Needs input",
    copy: "Waiting for your input",
    tone: "attention",
  },
  cancelling: {
    label: "Stopping",
    copy: "Stopping safely",
    tone: "attention",
  },
  completed: {
    label: "Completed",
    copy: "Work completed",
    tone: "success",
  },
  failed: {
    label: "Failed",
    copy: "Work failed",
    tone: "danger",
  },
  cancelled: {
    label: "Cancelled",
    copy: "Work cancelled",
    tone: "neutral",
  },
  interrupted: {
    label: "Interrupted",
    copy: "Work was interrupted",
    tone: "attention",
  },
  outcome_unknown: {
    label: "Outcome unknown",
    copy: "Verify the external outcome before retrying",
    tone: "danger",
  },
};

const TOOL_STATE_PRESENTATIONS: Readonly<
  Record<ToolActivityState, StatusPresentation>
> = {
  requested: {
    label: "Requested",
    copy: "Tool requested",
    tone: "neutral",
  },
  waiting_approval: {
    label: "Needs approval",
    copy: "Tool is waiting for approval",
    tone: "attention",
  },
  started: {
    label: "Running",
    copy: "Tool is running",
    tone: "progress",
  },
  completed: {
    label: "Completed",
    copy: "Tool completed",
    tone: "success",
  },
  cancelled: {
    label: "Cancelled",
    copy: "Tool did not start",
    tone: "neutral",
  },
  failed: {
    label: "Failed",
    copy: "Tool failed",
    tone: "danger",
  },
  outcome_unknown: {
    label: "Outcome unknown",
    copy: "Verify the tool outcome before retrying",
    tone: "danger",
  },
};

const GROUP_LABELS: Readonly<Record<WorkGroupKey, string>> = {
  pinned: "Pinned",
  attention: "Needs attention",
  active: "Active",
  recent: "Recent",
};

const GROUP_ORDER: readonly WorkGroupKey[] = [
  "pinned",
  "attention",
  "active",
  "recent",
];

function boundedLimit(
  requested: number | undefined,
  fallback: number,
  maximum: number,
): number {
  if (requested === undefined || !Number.isFinite(requested)) {
    return fallback;
  }
  return Math.min(maximum, Math.max(0, Math.trunc(requested)));
}

/**
 * Produces one-line renderer copy that cannot inject control/format characters or
 * unbounded labels into dense navigation and activity surfaces.
 */
export function safeDisplayLabel(
  value: string,
  fallback: string,
  maxCharacters = MAX_LABEL_CHARACTERS,
): string {
  const safeMaximum = Number.isFinite(maxCharacters)
    ? Math.max(1, Math.min(MAX_DETAIL_CHARACTERS, Math.trunc(maxCharacters)))
    : MAX_LABEL_CHARACTERS;
  const clean = (input: string) =>
    input
      .replace(/[\p{Cc}\p{Cf}]/gu, " ")
      .replace(/\s+/gu, " ")
      .trim();
  const candidate = clean(value) || clean(fallback);
  if (candidate.length <= safeMaximum) {
    return candidate;
  }

  const suffix = "…";
  let end = Math.max(0, safeMaximum - suffix.length);
  if (
    end > 0 &&
    /[\uD800-\uDBFF]/u.test(candidate.charAt(end - 1)) &&
    /[\uDC00-\uDFFF]/u.test(candidate.charAt(end))
  ) {
    end -= 1;
  }
  return candidate.slice(0, end).trimEnd() + suffix;
}

function humanizeIdentifier(
  value: string,
  fallback: string,
  maxCharacters = MAX_LABEL_CHARACTERS,
): string {
  const readable = value.replace(/[_-]+/gu, " ");
  const label = safeDisplayLabel(readable, fallback, maxCharacters);
  if (!/^[a-z\d ]+$/u.test(label)) {
    return label;
  }
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function presentRunStatus(status: RunStatus): StatusPresentation {
  return STATUS_PRESENTATIONS[status];
}

export function presentToolState(state: ToolActivityState): StatusPresentation {
  return TOOL_STATE_PRESENTATIONS[state];
}

export function runModeLabel(mode: RunMode): string {
  return mode === "plan"
    ? "Plan"
    : mode === "research"
      ? "Research"
      : "Execute";
}

export function agentRoleLabel(role: string): string {
  return humanizeIdentifier(role, "Default agent", 56);
}

export function shortDateLabel(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) {
    return "Recent";
  }
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function timestampValue(timestamp: string): number {
  const value = new Date(timestamp).getTime();
  return Number.isNaN(value) ? 0 : value;
}

function workGroupFor(run: Run, pinned: boolean): WorkGroupKey {
  if (pinned) {
    return "pinned";
  }
  if (
    run.status === "waiting" ||
    run.status === "outcome_unknown" ||
    run.pendingInteractionCount > 0
  ) {
    return "attention";
  }
  if (ACTIVE_STATUSES.has(run.status)) {
    return "active";
  }
  return "recent";
}

function toRecentWorkItem(run: Run): RecentWorkItem {
  const status = presentRunStatus(run.status);
  const isActive = ACTIVE_STATUSES.has(run.status);
  return {
    runId: run.runId,
    title: safeDisplayLabel(run.title, agentRoleLabel(run.role), 96),
    mode: run.mode,
    modeLabel: runModeLabel(run.mode),
    status: run.status,
    statusLabel: status.label,
    statusCopy: status.copy,
    statusTone: status.tone,
    updatedAt: run.updatedAt,
    updatedLabel: shortDateLabel(run.updatedAt),
    isActive,
    needsAttention:
      run.status === "waiting" ||
      run.status === "outcome_unknown" ||
      run.pendingInteractionCount > 0,
  };
}

function collapseSessionRuns(runs: readonly Run[]): Run[] {
  const sessions = new Map<
    string,
    { opening: Run; latest: Run; firstIndex: number }
  >();
  for (const [index, run] of runs
    .slice(0, MAX_PRESENTED_WORK_ITEMS)
    .entries()) {
    const current = sessions.get(run.sessionId);
    if (current === undefined) {
      sessions.set(run.sessionId, {
        opening: run,
        latest: run,
        firstIndex: index,
      });
      continue;
    }
    const opening =
      timestampValue(run.createdAt) < timestampValue(current.opening.createdAt)
        ? run
        : current.opening;
    const latest =
      timestampValue(run.updatedAt) > timestampValue(current.latest.updatedAt)
        ? run
        : current.latest;
    sessions.set(run.sessionId, {
      opening,
      latest,
      firstIndex: current.firstIndex,
    });
  }
  return [...sessions.values()]
    .sort((left, right) => left.firstIndex - right.firstIndex)
    .map(({ opening, latest }) => ({
      ...latest,
      title: opening.title,
    }));
}

/**
 * Groups durable sessions as work without exposing opaque session identities.
 * A continuation keeps the opening title while status and recency follow its
 * latest run.
 */
export function selectRecentWork(
  runs: readonly Run[],
  options: RecentWorkOptions = {},
): RecentWorkGroup[] {
  const limit = boundedLimit(
    options.limit,
    MAX_PRESENTED_WORK_ITEMS,
    MAX_PRESENTED_WORK_ITEMS,
  );
  const query = safeDisplayLabel(
    options.query ?? "",
    "",
    128,
  ).toLocaleLowerCase();
  const seen = new Set<string>();
  const groups = new Map<WorkGroupKey, RecentWorkItem[]>();
  const sorted = collapseSessionRuns(runs)
    .map((run, index) => ({ run, index }))
    .sort(
      (left, right) =>
        timestampValue(right.run.updatedAt) -
          timestampValue(left.run.updatedAt) || left.index - right.index,
    );

  let count = 0;
  for (const { run } of sorted) {
    if (count >= limit || seen.has(run.runId)) {
      continue;
    }
    seen.add(run.runId);
    const item = toRecentWorkItem(run);
    const searchable = [
      item.title,
      item.modeLabel,
      item.statusLabel,
      item.statusCopy,
    ]
      .join(" ")
      .toLocaleLowerCase();
    if (query !== "" && !searchable.includes(query)) {
      continue;
    }

    const key = workGroupFor(
      run,
      options.pinnedSessionIds?.has(run.sessionId) === true,
    );
    const items = groups.get(key) ?? [];
    items.push(item);
    groups.set(key, items);
    count += 1;
  }

  return GROUP_ORDER.flatMap((key) => {
    const items = groups.get(key);
    return items === undefined
      ? []
      : [{ key, label: GROUP_LABELS[key], items }];
  });
}

function isRunView(
  source: Exclude<RunViewSource, null | undefined>,
): source is RunView {
  return "run" in source && "updates" in source;
}

function collectViews(source: RunViewSource): RunView[] {
  if (source === null || source === undefined) {
    return [];
  }
  if (isRunView(source)) {
    return [source];
  }

  const views: RunView[] = [];
  for (const view of source) {
    views.push(view);
    if (views.length >= MAX_PRESENTED_VIEWS) {
      break;
    }
  }
  return views;
}

interface PresentedUpdate {
  view: RunView;
  update: RunUpdate;
  sourceIndex: number;
}

function presentedUpdates(source: RunViewSource): PresentedUpdate[] {
  return collectViews(source)
    .flatMap((view) =>
      view.updates
        .slice(-MAX_PRESENTED_UPDATES_PER_VIEW)
        .map((update, sourceIndex) => ({
          view,
          update,
          sourceIndex,
        })),
    )
    .sort(
      (left, right) =>
        timestampValue(right.update.createdAt) -
          timestampValue(left.update.createdAt) ||
        right.update.sequence - left.update.sequence ||
        left.sourceIndex - right.sourceIndex,
    );
}

export function formatByteSize(sizeBytes: number): string {
  if (!Number.isFinite(sizeBytes) || sizeBytes < 0) {
    return "Size unavailable";
  }
  if (sizeBytes < 1024) {
    return `${Math.trunc(sizeBytes)} B`;
  }
  if (sizeBytes < 1024 * 1024) {
    return `${(sizeBytes / 1024).toFixed(1)} KB`;
  }
  if (sizeBytes < 1024 * 1024 * 1024) {
    return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(sizeBytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function artifactTypeLabel(mediaType: string): string {
  const normalized = mediaType.toLocaleLowerCase();
  if (normalized === "application/pdf") {
    return "PDF";
  }
  if (normalized === "application/json") {
    return "JSON";
  }
  if (normalized.startsWith("image/")) {
    return "Image";
  }
  if (normalized.startsWith("audio/")) {
    return "Audio";
  }
  if (normalized.startsWith("video/")) {
    return "Video";
  }
  if (normalized.startsWith("text/")) {
    return "Text";
  }
  return "File";
}

function artifactStateLabel(state: ArtifactReference["state"]): string {
  switch (state) {
    case "uploading":
      return "Uploading";
    case "quarantined":
      return "Pending review";
    case "available":
      return "Available";
    case "rejected":
      return "Rejected";
    case "expired":
      return "Expired";
  }
}

function artifactPurposeLabel(purpose: ArtifactReference["purpose"]): string {
  switch (purpose) {
    case "run_input":
      return "Input";
    case "run_output":
      return "Output";
    case "workflow":
      return "Workflow";
    case "extension":
      return "Extension";
    case "archive":
      return "Archive";
  }
}

/**
 * Selects only authorized artifact metadata carried by released messages. The
 * digest and message text are intentionally absent from the presentation object.
 */
export function selectReleasedArtifacts(
  source: RunViewSource,
  requestedLimit = MAX_PRESENTED_ARTIFACTS,
): PresentedArtifact[] {
  const limit = boundedLimit(
    requestedLimit,
    MAX_PRESENTED_ARTIFACTS,
    MAX_PRESENTED_ARTIFACTS,
  );
  const artifacts: PresentedArtifact[] = [];
  const seen = new Set<string>();

  for (const { view, update } of presentedUpdates(source)) {
    if (artifacts.length >= limit) {
      break;
    }
    if (update.update.type !== "message") {
      continue;
    }
    for (
      let index = update.update.message.content.length - 1;
      index >= 0;
      index -= 1
    ) {
      if (artifacts.length >= limit) {
        break;
      }
      const part = update.update.message.content[index];
      if (part?.type !== "artifact") {
        continue;
      }
      const artifact = part.artifact;
      const identity =
        artifact.artifactId || `${view.run.runId}:${update.sequence}:${index}`;
      if (seen.has(identity)) {
        continue;
      }
      seen.add(identity);
      const sizeBytes =
        Number.isFinite(artifact.sizeBytes) && artifact.sizeBytes >= 0
          ? artifact.sizeBytes
          : 0;
      artifacts.push({
        artifactId: artifact.artifactId,
        key: identity,
        fileName: safeDisplayLabel(
          artifact.fileName,
          "Untitled artifact",
          MAX_FILE_NAME_CHARACTERS,
        ),
        mediaType: safeDisplayLabel(artifact.mediaType, "", 96),
        typeLabel: artifactTypeLabel(artifact.mediaType),
        sizeBytes,
        sizeLabel: formatByteSize(artifact.sizeBytes),
        purpose: artifact.purpose,
        purposeLabel: artifactPurposeLabel(artifact.purpose),
        state: artifact.state,
        stateLabel: artifactStateLabel(artifact.state),
        canOpen: artifact.state === "available",
        createdAt: artifact.createdAt,
        createdLabel: shortDateLabel(artifact.createdAt),
        runId: view.run.runId,
      });
    }
  }

  return artifacts;
}
