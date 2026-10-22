import type { Thread } from "./api";
import { chatThreadHref, threadHref } from "./routes";

type ConversationLocation = Pick<
  Thread,
  "project_id" | "thread_id" | "node_id" | "owner_user_id"
>;

/** A presentation hint only; the server authorizes every conversation read/action. */
export function isPersonalHostedChat(thread: ConversationLocation): boolean {
  return (
    Boolean(thread.owner_user_id) && /^chat-[a-f0-9]{64}$/.test(thread.node_id)
  );
}

export function conversationHref(thread: ConversationLocation): string {
  return isPersonalHostedChat(thread)
    ? chatThreadHref(thread.project_id, thread.thread_id)
    : threadHref(thread.project_id, thread.thread_id);
}
