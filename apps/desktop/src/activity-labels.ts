import { toolActivityLabel } from "@colossus/ui/lib/tool-activity";
import { safeDisplayLabel } from "./presenters";
import type { ToolActivity } from "./types";

export type ActivityLabelKind =
  | "browser"
  | "delegate"
  | "generic"
  | "image"
  | "list"
  | "mcp"
  | "read"
  | "run"
  | "search"
  | "web"
  | "write";

export interface ActivityLabel {
  title: string;
  kind: ActivityLabelKind;
}

export interface NoticeLabel {
  title: string;
  detail: string;
  kind: "handoff" | "info" | "research";
}

export function presentToolActivity(activity: ToolActivity): ActivityLabel {
  return {
    title: toolActivityLabel(activity.summary, activity.toolName),
    kind: toolActivityKind(activity.toolName),
  };
}

// Icons describe the registered tool family, independently of released summary text.
// Never derive a label or icon from tool arguments or output.
function toolActivityKind(toolName: string): ActivityLabelKind {
  switch (toolName) {
    case "image.view":
      return "image";
    case "filesystem.read":
    case "fs.read":
    case "patch.preview":
    case "repo.read_many":
    case "repo.file_summary":
    case "workspace.inspect":
      return "read";
    case "filesystem.write":
    case "fs.write":
    case "filesystem.replace":
    case "workspace.apply_patch":
    case "patch.apply":
    case "patch.reverse":
      return "write";
    case "filesystem.list":
    case "fs.list":
    case "repo.map":
    case "repo.map_structure":
      return "list";
    case "filesystem.search":
    case "fs.search":
    case "repo.search":
    case "repo.symbol_search":
    case "repo.references":
      return "search";
    case "web.search":
    case "web.fetch":
    case "docs.fetch":
      return "web";
    case "shell.run":
    case "shell.read":
    case "shell.wait":
    case "shell.list":
    case "shell.stop":
    case "process.spawn":
    case "process.start":
    case "process.poll":
    case "process.read":
    case "process.write":
    case "process.stop":
    case "process.list":
      return "run";
    case "agent.delegate":
    case "agent.result":
    case "agent.subagent_update":
      return "delegate";
    default:
      return toolName.startsWith("mcp.")
        ? "mcp"
        : toolName.startsWith("browser.")
          ? "browser"
          : "generic";
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
