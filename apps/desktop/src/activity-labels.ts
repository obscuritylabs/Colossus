import { safeDisplayLabel } from "./presenters";
import type { ToolActivity, ToolActivityState } from "./types";

export type ActivityLabelKind =
  "delegate" | "generic" | "list" | "read" | "run" | "search" | "web" | "write";

export interface ActivityLabel {
  title: string;
  kind: ActivityLabelKind;
}

export interface NoticeLabel {
  title: string;
  detail: string;
  kind: "handoff" | "info" | "research";
}

type InputObject = Record<string, unknown>;

interface ActionForms {
  requested: string;
  active: string;
  completed: string;
  cancelled: string;
  failed: string;
}

function parsedInput(value: string | null | undefined): InputObject | null {
  if (!value?.trim()) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(value);
    return typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed)
      ? (parsed as InputObject)
      : null;
  } catch {
    return null;
  }
}

function stringValue(input: InputObject | null, ...keys: string[]): string {
  for (const key of keys) {
    const value = input?.[key];
    if (typeof value === "string" && value.trim()) {
      return safeDisplayLabel(value, "", 72);
    }
  }
  return "";
}

function stringList(input: InputObject | null, key: string): string[] {
  const value = input?.[key];
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .filter((item): item is string => typeof item === "string")
    .map((item) => safeDisplayLabel(item, "", 72))
    .filter(Boolean);
}

function shortPath(value: string): string {
  const normalized = value.replaceAll("\\", "/").replace(/\/+$/u, "");
  if (normalized === "" || normalized === ".") {
    return "the workspace";
  }
  return safeDisplayLabel(
    normalized.split("/").at(-1) ?? normalized,
    value,
    56,
  );
}

function quoted(value: string): string {
  return `“${safeDisplayLabel(value, "", 64)}”`;
}

function actionForState(state: ToolActivityState, forms: ActionForms): string {
  switch (state) {
    case "requested":
    case "waiting_approval":
      return forms.requested;
    case "started":
      return forms.active;
    case "completed":
      return forms.completed;
    case "cancelled":
      return forms.cancelled;
    case "failed":
    case "outcome_unknown":
      return forms.failed;
  }
}

function fileAction(
  state: ToolActivityState,
  path: string,
  verbs: {
    request: string;
    active: string;
    complete: string;
    cancelled: string;
    failed: string;
  },
): string {
  const target = path === "" ? "workspace files" : shortPath(path);
  return actionForState(state, {
    requested: `${verbs.request} ${target}`,
    active: `${verbs.active} ${target}`,
    completed: `${verbs.complete} ${target}`,
    cancelled: `${verbs.cancelled} ${target}`,
    failed: `${verbs.failed} ${target}`,
  });
}

function searchAction(
  state: ToolActivityState,
  query: string,
  location = "",
): string {
  const target =
    query === ""
      ? location
      : `${quoted(query)}${location === "" ? "" : ` in ${location}`}`;
  const suffix = target === "" ? "" : ` ${target}`;
  return actionForState(state, {
    requested: `Preparing to search${suffix}`,
    active: `Searching${suffix}`,
    completed: `Searched${suffix}`,
    cancelled: `Skipped searching${suffix}`,
    failed: `Couldn’t search${suffix}`,
  });
}

function webSearchAction(state: ToolActivityState, query: string): string {
  const target = query === "" ? "" : ` for ${quoted(query)}`;
  return actionForState(state, {
    requested: `Preparing to search the web${target}`,
    active: `Searching the web${target}`,
    completed: `Searched the web${target}`,
    cancelled: `Skipped searching the web${target}`,
    failed: `Couldn’t search the web${target}`,
  });
}

function commandAction(state: ToolActivityState, command: string): string {
  const suffix = command === "" ? " a command" : ` ${command}`;
  return actionForState(state, {
    requested: `Preparing to run${suffix}`,
    active: `Running${suffix}`,
    completed: `Ran${suffix}`,
    cancelled: `Skipped running${suffix}`,
    failed: `Couldn’t run${suffix}`,
  });
}

function listAction(state: ToolActivityState, path: string): string {
  const location = path === "" ? "" : shortPath(path);
  const target =
    location === "" || location === "the workspace"
      ? "workspace files"
      : `files in ${location}`;
  return actionForState(state, {
    requested: `Preparing to list ${target}`,
    active: `Listing ${target}`,
    completed: `Listed ${target}`,
    cancelled: `Skipped listing ${target}`,
    failed: `Couldn’t list ${target}`,
  });
}

function genericToolTitle(activity: ToolActivity): string {
  const summary = safeDisplayLabel(activity.summary, "", 96);
  const generic =
    /^(validated tool call requested|tool execution (requested|started|completed|cancelled|failed|was cancelled)( at turn \d+)?)/iu.test(
      summary,
    );
  if (summary !== "" && !generic) {
    return summary;
  }
  const name = safeDisplayLabel(
    activity.toolName.replace(/[._-]+/gu, " "),
    "tool",
    64,
  );
  return actionForState(activity.state, {
    requested: `Preparing ${name}`,
    active: `Using ${name}`,
    completed: `Used ${name}`,
    cancelled: `Skipped ${name}`,
    failed: `Couldn’t use ${name}`,
  });
}

export function presentToolActivity(
  activity: ToolActivity,
  releasedInput: string | null = activity.input ?? null,
): ActivityLabel {
  const input = parsedInput(releasedInput);
  const path = stringValue(input, "path", "file", "cwd", "root");
  const query = stringValue(input, "query", "pattern", "symbol");
  const command =
    stringValue(input, "command", "cmd") ||
    safeDisplayLabel(stringList(input, "argv").join(" "), "", 96);

  switch (activity.toolName) {
    case "filesystem.read":
      return {
        kind: "read",
        title: fileAction(activity.state, path, {
          request: "Preparing to read",
          active: "Reading",
          complete: "Read",
          cancelled: "Skipped reading",
          failed: "Couldn’t read",
        }),
      };
    case "filesystem.write": {
      const creating = stringValue(input, "mode") === "create";
      return {
        kind: "write",
        title: fileAction(activity.state, path, {
          request: creating ? "Preparing to create" : "Preparing to update",
          active: creating ? "Creating" : "Updating",
          complete: creating ? "Created" : "Updated",
          cancelled: creating ? "Skipped creating" : "Skipped updating",
          failed: creating ? "Couldn’t create" : "Couldn’t update",
        }),
      };
    }
    case "filesystem.replace":
    case "workspace.apply_patch":
    case "patch.apply":
      return {
        kind: "write",
        title: fileAction(activity.state, path, {
          request: "Preparing to update",
          active: "Updating",
          complete: "Updated",
          cancelled: "Skipped updating",
          failed: "Couldn’t update",
        }),
      };
    case "patch.preview":
      return {
        kind: "read",
        title: fileAction(activity.state, path, {
          request: "Preparing to preview changes to",
          active: "Previewing changes to",
          complete: "Previewed changes to",
          cancelled: "Skipped previewing changes to",
          failed: "Couldn’t preview changes to",
        }),
      };
    case "patch.reverse":
      return {
        kind: "write",
        title: fileAction(activity.state, path, {
          request: "Preparing to revert",
          active: "Reverting",
          complete: "Reverted",
          cancelled: "Skipped reverting",
          failed: "Couldn’t revert",
        }),
      };
    case "filesystem.list":
      return {
        kind: "list",
        title: listAction(activity.state, path),
      };
    case "filesystem.search": {
      const location = path === "" ? "" : shortPath(path);
      return {
        kind: "search",
        title: searchAction(activity.state, query, location),
      };
    }
    case "repo.search":
      return { kind: "search", title: searchAction(activity.state, query) };
    case "repo.symbol_search":
      return {
        kind: "search",
        title: searchAction(activity.state, query, "repository symbols"),
      };
    case "repo.references":
      return {
        kind: "search",
        title: searchAction(activity.state, query, "symbol references"),
      };
    case "web.search":
      return {
        kind: "web",
        title: webSearchAction(activity.state, query),
      };
    case "web.fetch":
    case "docs.fetch": {
      const url = stringValue(input, "url", "uri");
      return {
        kind: "web",
        title: fileAction(activity.state, url, {
          request: "Preparing to open",
          active: "Opening",
          complete: "Opened",
          cancelled: "Skipped opening",
          failed: "Couldn’t open",
        }),
      };
    }
    case "repo.map":
    case "repo.map_structure":
      return {
        kind: "list",
        title: actionForState(activity.state, {
          requested: "Preparing to map the repository",
          active: "Mapping the repository",
          completed: "Mapped repository structure",
          cancelled: "Skipped repository mapping",
          failed: "Couldn’t map the repository",
        }),
      };
    case "repo.read_many": {
      const paths = stringList(input, "paths");
      if (paths.length === 1) {
        return {
          kind: "read",
          title: fileAction(activity.state, paths[0] ?? "", {
            request: "Preparing to read",
            active: "Reading",
            complete: "Read",
            cancelled: "Skipped reading",
            failed: "Couldn’t read",
          }),
        };
      }
      const target = paths.length > 0 ? `${paths.length} files` : "files";
      return {
        kind: "read",
        title: actionForState(activity.state, {
          requested: `Preparing to read ${target}`,
          active: `Reading ${target}`,
          completed: `Read ${target}`,
          cancelled: `Skipped reading ${target}`,
          failed: `Couldn’t read ${target}`,
        }),
      };
    }
    case "repo.file_summary":
      return {
        kind: "read",
        title: fileAction(activity.state, path, {
          request: "Preparing to review",
          active: "Reviewing",
          complete: "Reviewed",
          cancelled: "Skipped reviewing",
          failed: "Couldn’t review",
        }),
      };
    case "shell.run":
    case "process.spawn":
      return { kind: "run", title: commandAction(activity.state, command) };
    case "workspace.inspect":
      return {
        kind: "read",
        title: actionForState(activity.state, {
          requested: "Preparing to inspect the workspace",
          active: "Inspecting the workspace",
          completed: "Inspected the workspace",
          cancelled: "Skipped workspace inspection",
          failed: "Couldn’t inspect the workspace",
        }),
      };
    case "agent.delegate":
      return {
        kind: "delegate",
        title: actionForState(activity.state, {
          requested: "Preparing delegated work",
          active: "Delegating work",
          completed: "Delegated work",
          cancelled: "Skipped delegated work",
          failed: "Couldn’t delegate work",
        }),
      };
    case "agent.result":
      return { kind: "delegate", title: "Collected delegated findings" };
    default:
      return { kind: "generic", title: genericToolTitle(activity) };
  }
}

function titleCaseIdentifier(value: string): string {
  const label = safeDisplayLabel(
    value.replace(/[._-]+/gu, " "),
    "Activity update",
    80,
  );
  return label.charAt(0).toUpperCase() + label.slice(1);
}

function normalizedNoticeDetail(reason: string, message: string): string {
  if (reason === "research.collecting.skipped") {
    return "Source or worker limit reached.";
  }
  if (reason === "research.planning.started") {
    return "Preparing focused, bounded research queries.";
  }
  if (reason === "research.planning.completed") {
    return "Research queries are ready.";
  }
  if (reason === "research.synthesis.started") {
    return "Combining findings into a citation-backed report.";
  }
  if (reason === "research.synthesis.completed") {
    return "The citation-backed report is ready.";
  }
  const releasedSources = /^released (\d+) (.+?) source\(s\)$/iu.exec(
    message.trim(),
  );
  if (releasedSources !== null) {
    const count = Number(releasedSources[1]);
    return `Added ${releasedSources[1]} ${releasedSources[2]} source${count === 1 ? "" : "s"}.`;
  }
  const detail = safeDisplayLabel(message, "Activity updated", 180);
  return detail.charAt(0).toUpperCase() + detail.slice(1);
}

const RESEARCH_NOTICE_TITLES: Readonly<Record<string, string>> = {
  "research.planning.started": "Planning research",
  "research.planning.completed": "Research plan ready",
  "research.planning.fallback": "Research plan ready with fallback",
  "research.planning.skipped": "Research planning skipped",
  "research.planning.failed": "Research planning failed",
  "research.collecting.started": "Gathering sources",
  "research.collecting.completed": "Gathered sources",
  "research.collecting.fallback": "Gathered sources with fallback",
  "research.collecting.skipped": "Source search skipped",
  "research.collecting.failed": "Source search failed",
  "research.workers.started": "Reviewing source evidence",
  "research.workers.completed": "Reviewed source evidence",
  "research.workers.fallback": "Reviewed evidence with fallback",
  "research.workers.skipped": "Source review skipped",
  "research.workers.failed": "Source review failed",
  "research.synthesis.started": "Writing research report",
  "research.synthesis.completed": "Research report ready",
  "research.synthesis.fallback": "Research report ready with fallback",
  "research.synthesis.skipped": "Report writing skipped",
  "research.synthesis.failed": "Report writing failed",
  "research.recovery.started": "Checking interrupted research",
  "research.recovery.completed": "Recovered interrupted research",
  "research.recovery.fallback": "Recovered research with fallback",
  "research.recovery.skipped": "Research recovery skipped",
  "research.recovery.failed": "Research recovery failed",
};

export function presentNotice(reason: string, message: string): NoticeLabel {
  const researchTitle = RESEARCH_NOTICE_TITLES[reason];
  if (researchTitle !== undefined) {
    return {
      title: researchTitle,
      detail: normalizedNoticeDetail(reason, message),
      kind: "research",
    };
  }
  const handoff = reason.includes("handoff");
  return {
    title: handoff ? "Agent handoff complete" : titleCaseIdentifier(reason),
    detail: normalizedNoticeDetail(reason, message),
    kind: handoff ? "handoff" : "info",
  };
}
