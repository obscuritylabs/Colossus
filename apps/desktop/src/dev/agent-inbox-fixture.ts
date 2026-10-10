import type {
  InboxMessage,
  InboxPage,
  InboxParticipant,
} from "@colossus/ui/agent-inbox";

export function buildAgentInboxParticipants(
  rootRunId: string,
): InboxParticipant[] {
  const root: InboxParticipant = {
    id: `${rootRunId}:root`,
    root_run_id: rootRunId,
    session_id: "fixture-session-root",
    run_id: rootRunId,
    parent_id: null,
    subagent_id: null,
    generation: 1,
    open: true,
    closed_reason: null,
    pending_messages: 1,
    pending_bytes: 23,
    created_at: "2026-10-09T12:00:00Z",
  };
  return [
    root,
    {
      ...root,
      id: `${rootRunId}:child-1`,
      parent_id: root.id,
      session_id: "fixture-session-child",
      subagent_id: "fixture-job-builder",
      run_id: "fixture-child-run",
      open: false,
      closed_reason: "interrupted",
      pending_messages: 0,
      pending_bytes: 0,
    },
    {
      ...root,
      id: `${rootRunId}:child-2`,
      parent_id: root.id,
      session_id: "fixture-session-child",
      subagent_id: "fixture-job-builder",
      run_id: null,
      generation: 2,
      pending_messages: 0,
      pending_bytes: 0,
    },
  ];
}
export function buildAgentInboxMessages(
  participantId: string,
  after: number,
): InboxPage {
  const rootRunId = participantId.slice(0, participantId.lastIndexOf(":"));
  if (participantId.endsWith("child-2"))
    return { messages: [], next_sequence: 0, has_more: false };
  const messages: InboxMessage[] = Array.from({ length: 19 }, (_, index) => ({
    id: `fixture-message-${index + 1}`,
    root_run_id: rootRunId,
    sender: { kind: "application", application_id: "desktop-managed-local" },
    recipient_id: participantId,
    sequence: index + 1,
    text:
      index === 0
        ? "Please verify the revised requirements.\n<script>Untrusted peer text stays visible as text.</script>"
        : `Peer update ${index + 1}: review the current workspace and report your findings.`,
    reply_to: index === 1 ? "fixture-message-1" : null,
    accepted_at: "2026-10-09T12:01:00Z",
    receipt:
      index === 1
        ? { state: "accepted" }
        : index === 2
          ? { state: "not_delivered", reason: "interrupted" }
          : {
              state: "included_in_turn",
              run_id: participantId.endsWith(":root")
                ? rootRunId
                : "fixture-child-run",
              turn: 2,
              request_hash: "a".repeat(64),
            },
  }));
  const page = messages.slice(after, after + 16);
  return {
    messages: page,
    next_sequence: page.at(-1)?.sequence ?? after,
    has_more: after + page.length < messages.length,
  };
}
