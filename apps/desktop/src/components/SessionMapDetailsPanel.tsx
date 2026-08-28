import {
  IconBook2,
  IconChecklist,
  IconFileText,
  IconRobot,
  IconScale,
  IconSearch,
  IconTargetArrow,
  IconAdjustments,
} from "@tabler/icons-react";
import type { ComponentType, ReactNode } from "react";

import { shortDateLabel } from "../presenters";
import type { SessionMapResource } from "../types";

function readable(value: string): string {
  return value
    .replaceAll("_", " ")
    .replace(/^./, (letter) => letter.toUpperCase());
}

function resourceTitle(resource: SessionMapResource): string {
  switch (resource.family) {
    case "delegates":
      return resource.value.task;
    case "goals":
      return resource.value.objective;
    case "tasks":
      return resource.value.title;
    case "plans":
      return resource.value.prompt;
    case "decisions":
      return resource.value.title;
    case "memories":
      return resource.value.text;
    case "snapshots":
      return (
        resource.value.summary ||
        `Messages ${resource.value.sourceStartSequence}–${resource.value.sourceEndSequence}`
      );
    case "research":
      return resource.value.question;
    case "sources":
      return resource.value.title;
  }
}

function resourceStatus(resource: SessionMapResource): string {
  if (resource.family === "sources") return "released";
  if (resource.family === "snapshots") return "immutable";
  return resource.value.status;
}

function resourceUpdatedAt(resource: SessionMapResource): string {
  return resource.family === "sources" || resource.family === "snapshots"
    ? resource.value.createdAt
    : resource.value.updatedAt;
}

const FAMILY_META: Record<
  SessionMapResource["family"],
  { label: string; icon: ComponentType<{ size?: number; stroke?: number }> }
> = {
  delegates: { label: "Delegated agent", icon: IconRobot },
  goals: { label: "Goal", icon: IconTargetArrow },
  tasks: { label: "Task", icon: IconChecklist },
  plans: { label: "Plan", icon: IconFileText },
  decisions: { label: "Key decision", icon: IconScale },
  memories: { label: "Memory", icon: IconBook2 },
  snapshots: { label: "Context snapshot", icon: IconAdjustments },
  research: { label: "Research", icon: IconSearch },
  sources: { label: "Source", icon: IconFileText },
};

function Detail({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="session-map-detail-field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

export function SessionMapDetailsPanel({
  resource,
  spaceName,
  onBack,
}: {
  resource: SessionMapResource;
  spaceName: string;
  onBack: () => void;
}) {
  const meta = FAMILY_META[resource.family];
  const Icon = meta.icon;
  const title = resourceTitle(resource);
  const status = resourceStatus(resource);
  return (
    <section
      className="session-map-details"
      aria-labelledby="session-map-detail-title"
    >
      <header>
        <button
          type="button"
          onClick={onBack}
          aria-label="Back to thread details"
        >
          ← <span>Thread details</span>
        </button>
        <span className={`session-map-detail-status status-${status}`}>
          {readable(status)}
        </span>
      </header>
      <div className="session-map-detail-heading">
        <span aria-hidden="true">
          <Icon size={22} stroke={1.55} />
        </span>
        <div>
          <p>{meta.label}</p>
          <h3 id="session-map-detail-title">{title}</h3>
        </div>
      </div>

      <dl>
        <Detail label="Workspace">{spaceName}</Detail>
        <Detail label="Status">{readable(status)}</Detail>
        <Detail label="Updated">
          {shortDateLabel(resourceUpdatedAt(resource))}
        </Detail>
        {resource.family === "memories" ? (
          <>
            <Detail label="Kind">{resource.value.kind}</Detail>
            <Detail label="Scope">{readable(resource.value.scope)}</Detail>
            <Detail label="Confidence">
              {Math.round(resource.value.confidence * 100)}%
            </Detail>
            <Detail label="Source">{readable(resource.value.source)}</Detail>
          </>
        ) : null}
        {resource.family === "snapshots" ? (
          <>
            <Detail label="Strategy">
              {readable(resource.value.strategy)}
            </Detail>
            <Detail label="Message range">
              {resource.value.sourceStartSequence}–
              {resource.value.sourceEndSequence}
            </Detail>
            <Detail label="Pinned facts">
              {resource.value.pinnedFacts.length}
            </Detail>
            <Detail label="Open tasks">
              {resource.value.openTasks.length}
            </Detail>
          </>
        ) : null}
        {resource.family === "goals" ? (
          <Detail label="Iterations">
            {resource.value.iterationsCompleted} /{" "}
            {resource.value.iterationBudget}
          </Detail>
        ) : null}
        {resource.family === "plans" ? (
          <>
            <Detail label="Revision">{resource.value.revision}</Detail>
            <Detail label="Steps">{resource.value.stepCount}</Detail>
          </>
        ) : null}
        {resource.family === "decisions" ? (
          <>
            <Detail label="Priority">
              {readable(resource.value.priority)}
            </Detail>
            <Detail label="Source">{readable(resource.value.source)}</Detail>
          </>
        ) : null}
        {resource.family === "research" ? (
          <>
            <Detail label="Depth">{readable(resource.value.depth)}</Detail>
            <Detail label="Sources">{resource.value.sourceCount}</Detail>
            <Detail label="Queries">{resource.value.queryCount}</Detail>
          </>
        ) : null}
        {resource.family === "sources" ? (
          <>
            <Detail label="Label">{resource.value.label}</Detail>
            <Detail label="Kind">{readable(resource.value.kind)}</Detail>
            <Detail label="Location">
              <code>{resource.value.uri}</code>
            </Detail>
          </>
        ) : null}
      </dl>

      <div className="session-map-detail-copy">
        {resource.family === "delegates" ? (
          <>
            <h4>Objective</h4>
            <p>{resource.value.task}</p>
            {resource.value.finalOutput !== "" ? (
              <>
                <h4>Released result</h4>
                <p>{resource.value.finalOutput}</p>
              </>
            ) : null}
            {resource.value.error !== "" ? (
              <>
                <h4>Error</h4>
                <p>{resource.value.error}</p>
              </>
            ) : null}
          </>
        ) : null}
        {resource.family === "goals" ? (
          <>
            {resource.value.summary !== "" ? (
              <>
                <h4>Summary</h4>
                <p>{resource.value.summary}</p>
              </>
            ) : null}
            {resource.value.blockedReason !== "" ? (
              <>
                <h4>Blocked reason</h4>
                <p>{resource.value.blockedReason}</p>
              </>
            ) : null}
          </>
        ) : null}
        {resource.family === "tasks" ? (
          <>
            <h4>Description</h4>
            <p>{resource.value.description}</p>
          </>
        ) : null}
        {resource.family === "plans" ? (
          <>
            <h4>Released plan</h4>
            <pre>
              {resource.value.content || "No plan content was released."}
            </pre>
          </>
        ) : null}
        {resource.family === "decisions" ? (
          <>
            <h4>Decision</h4>
            <p>{resource.value.decision}</p>
            <h4>Intent</h4>
            <p>{resource.value.intent}</p>
            <h4>Applies when</h4>
            <p>{resource.value.appliesWhen}</p>
            <h4>Rationale</h4>
            <p>{resource.value.rationale}</p>
          </>
        ) : null}
        {resource.family === "memories" ? (
          <>
            <h4>Text</h4>
            <p>{resource.value.text}</p>
            <h4>Rationale</h4>
            <p>{resource.value.rationale}</p>
          </>
        ) : null}
        {resource.family === "snapshots" ? (
          <>
            <h4>Summary</h4>
            <p>{resource.value.summary || "No summary was recorded."}</p>
            <SnapshotList
              title="Pinned facts"
              items={resource.value.pinnedFacts}
            />
            <SnapshotList title="Open tasks" items={resource.value.openTasks} />
            <SnapshotList
              title="Files touched"
              items={resource.value.filesTouched}
            />
            <SnapshotList
              title="Notable tool results"
              items={resource.value.notableToolResults}
            />
          </>
        ) : null}
        {resource.family === "research" ? (
          <>
            <h4>Released report</h4>
            <pre>{resource.value.report || "No report was released."}</pre>
            {resource.value.error !== "" ? (
              <>
                <h4>Error</h4>
                <p>{resource.value.error}</p>
              </>
            ) : null}
          </>
        ) : null}
        {resource.family === "sources" ? (
          <>
            <h4>Query</h4>
            <p>{resource.value.query}</p>
          </>
        ) : null}
      </div>
    </section>
  );
}

function SnapshotList({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <>
      <h4>{title}</h4>
      <ul>
        {items.map((item, index) => (
          <li key={`${index}:${item}`}>{item}</li>
        ))}
      </ul>
    </>
  );
}
