import {
  AgentInboxInspector,
  type AgentInboxProps,
} from "@colossus/ui/agent-inbox";
import { useEffect, useState } from "react";
import type { RunView } from "../state";
import { DropdownSelect } from "./DropdownSelect";

export function AgentInboxesView({
  views,
  currentRunId,
  inbox,
}: {
  views: readonly RunView[];
  currentRunId: string;
  inbox: Omit<AgentInboxProps, "rootRunId"> | undefined;
}) {
  const [runId, setRunId] = useState(currentRunId);
  useEffect(() => setRunId(currentRunId), [currentRunId]);
  const selected = views.some((view) => view.run.runId === runId)
    ? runId
    : currentRunId;
  return (
    <>
      <label className="agent-inbox">
        Execution
        <DropdownSelect
          aria-label="Execution"
          value={selected}
          onChange={(event) => setRunId(event.target.value)}
        >
          {views.map((view) => (
            <option key={view.run.runId} value={view.run.runId}>
              {view.run.runId} · {view.run.status}
            </option>
          ))}
        </DropdownSelect>
      </label>
      <AgentInboxInspector
        rootRunId={selected}
        available={inbox?.available ?? false}
        loadParticipants={inbox?.loadParticipants}
        loadMessages={inbox?.loadMessages}
      />
    </>
  );
}
