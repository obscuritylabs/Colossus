import { toolActivityLabel } from "@colossus/ui/lib/tool-activity";
import { safeDisplayLabel } from "./presenters";
import type { ToolActivity } from "./types";

export type ActivityLabelKind = "browser" | "generic" | "mcp";

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
    kind: activity.toolName.startsWith("mcp.")
      ? "mcp"
      : activity.toolName.startsWith("browser.")
        ? "browser"
        : "generic",
  };
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
