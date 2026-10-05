export interface ScheduleExample {
  id: string;
  name: string;
  description: string;
  instructions: string;
  repeat: "daily" | "weekly";
  time: string;
  weekdays: number[];
  tools: string[];
}

export const scheduleExamples: ScheduleExample[] = [
  {
    id: "cyber-briefing",
    name: "Cybersecurity market briefing",
    description: "Monday morning · Policy, procurement, and agency priorities.",
    instructions:
      "Brief me on consequential U.S. federal cybersecurity market developments relevant to leading a cyber services firm. Prioritize procurement, policy, and agency priorities. Include source links, publication dates, and actionable implications. Clearly distinguish reported facts from interpretation.",
    repeat: "weekly",
    time: "09:00",
    weekdays: [1],
    tools: ["web.search"],
  },
  {
    id: "vulnerability-watch",
    name: "Vulnerability watch",
    description: "Weekday mornings · Relevant advisories and remediation.",
    instructions:
      "Review public vulnerability advisories affecting technologies documented in this workspace. Summarize urgency, affected versions, known exploitation, and remediation with source links. Flag uncertain exposure. Do not modify files, systems, or dependencies.",
    repeat: "weekly",
    time: "08:30",
    weekdays: [1, 2, 3, 4, 5],
    tools: ["filesystem.read", "filesystem.search", "web.search"],
  },
  {
    id: "workspace-health",
    name: "Workspace health check",
    description: "Every morning · Changes, unfinished work, and blockers.",
    instructions:
      "Review this workspace's Git status and current diff. Summarize changed files, unfinished work, and concrete risks or blockers. Recommend the next useful action. Do not edit files, run commands, commit, or publish anything.",
    repeat: "daily",
    time: "09:00",
    weekdays: [],
    tools: ["git.status", "git.diff"],
  },
  {
    id: "dependency-review",
    name: "Dependency review",
    description: "Wednesday mornings · Updates and security considerations.",
    instructions:
      "Inspect dependency manifests in this workspace and research notable security advisories or upstream deprecations. Summarize recommended updates with source links and compatibility considerations. Identify missing information. Do not install packages or modify manifests or lockfiles.",
    repeat: "weekly",
    time: "10:00",
    weekdays: [3],
    tools: ["filesystem.read", "filesystem.search", "web.search"],
  },
  {
    id: "release-notes",
    name: "Release notes draft",
    description: "Friday afternoon · A concise digest of recent changes.",
    instructions:
      "Review recent Git changes in this workspace and draft concise release notes grouped into features, fixes, and breaking changes. Link changes to the evidence available in the repository. Flag anything needing confirmation. Return the draft in the run result; do not edit files, commit, tag, or publish a release.",
    repeat: "weekly",
    time: "16:00",
    weekdays: [5],
    tools: ["git.status", "git.diff", "git.show"],
  },
  {
    id: "incident-readiness",
    name: "Incident readiness review",
    description: "Tuesday mornings · Runbook gaps and escalation readiness.",
    instructions:
      "Review incident response runbooks and escalation guidance available in this workspace. Identify missing steps, stale references, and unclear ownership. Give a prioritized readiness checklist with file references. Do not contact anyone, modify files, or execute operational actions.",
    repeat: "weekly",
    time: "09:00",
    weekdays: [2],
    tools: ["filesystem.read", "filesystem.search"],
  },
];

export function scheduleExamplePrompt(
  example: ScheduleExample,
  timezone: string,
): string {
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  const repeat =
    example.repeat === "daily"
      ? "Daily"
      : `Weekly on ${example.weekdays.map((day) => days[day - 1]).join(", ")}`;
  return `@colossus/schedule-task
Create a scheduled task in this workspace.

Name: ${example.name}
Instructions: ${example.instructions}
Repeat: ${repeat}
Local time: ${example.time}
Time zone: ${timezone}
Allowed task tools: ${example.tools.join(", ")}
Model and effort: workspace defaults
Missed runs: run latest once
Initial state: enabled

Choose the next future occurrence, preserve this local time through DST, and use workflow.task.schedule with one durable retry identity. Follow workspace permissions and approval rules. If scheduling or a requested tool is unavailable, explain what is missing. Confirm the stored schedule and report its next occurrence. The worker must be running for the task to execute.`;
}
