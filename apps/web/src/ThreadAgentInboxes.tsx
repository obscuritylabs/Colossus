import {
  AgentInboxInspector,
  type InboxPage,
  type InboxParticipant,
} from "@colossus/ui/agent-inbox";
import { useState } from "react";
import { request, projectPath, type Task } from "./api";

type Inspection = {
  kind: "inboxes";
  participants: InboxParticipant[];
  page: InboxPage | null;
};
type Receipt = {
  command_id: string;
  reply: Inspection | { kind: "failed"; error: { message: string } } | null;
};

export function ThreadAgentInboxes({
  project,
  task,
  available,
}: {
  project: string;
  task: Task;
  available: boolean;
}) {
  const [open, setOpen] = useState(false);
  const path = `${projectPath(project)}/tasks/${encodeURIComponent(task.task_id)}`;
  async function inspect(
    participantId?: string,
    afterSequence = 0,
  ): Promise<Inspection> {
    let command = (
      await request<{ command: Receipt }>(`${path}/inboxes`, {
        request_id: crypto.randomUUID(),
        participant_id: participantId ?? null,
        after_sequence: afterSequence,
      })
    ).command;
    const deadline = Date.now() + 10_000;
    while (!command.reply && Date.now() < deadline) {
      await new Promise<void>((resolve) => setTimeout(resolve, 300));
      command = (
        await request<{ command: Receipt }>(
          `${path}/commands/${encodeURIComponent(command.command_id)}`,
        )
      ).command;
    }
    if (!command.reply)
      throw new Error(
        "The connected runtime has not answered this inspection.",
      );
    if (command.reply.kind === "failed")
      throw new Error(command.reply.error.message);
    return command.reply;
  }
  if (!task.run_id) return null;
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>Agent inboxes</summary>
      {open && (
        <AgentInboxInspector
          rootRunId={task.run_id}
          available={available && !task.source_read_only}
          loadParticipants={async () => (await inspect()).participants}
          loadMessages={async (id, after) => {
            const value = await inspect(id, after);
            if (!value.page) throw new Error("Missing inbox page");
            return value.page;
          }}
        />
      )}
    </details>
  );
}
