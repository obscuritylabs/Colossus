import {
  taskStatus,
  terminalStatuses,
  type Task,
  type Thread,
  type ThreadDetailResponse,
} from "./api";

/** A returned row must still represent the same personal runtime binding. */
export function sameChatBinding(bound: Thread, current: Thread) {
  return (
    current.thread_id === bound.thread_id &&
    current.project_id === bound.project_id &&
    current.node_id === bound.node_id &&
    current.owner_user_id === bound.owner_user_id &&
    current.session_id === bound.session_id
  );
}

/** Detail tasks arrive newest first; inherited read-only runs cannot be fork sources. */
export function latestChatForkSource(
  bound: Thread,
  detail: ThreadDetailResponse,
  identity: string,
): Task | null {
  const thread = detail.thread;
  if (
    !sameChatBinding(bound, thread) ||
    thread.owner_user_id !== identity ||
    thread.archived ||
    !thread.can_continue ||
    thread.sync_status !== "current" ||
    !thread.session_id ||
    thread.active_task_id ||
    thread.queued_task_ids?.length ||
    detail.tasks.some(
      (task) =>
        !task.source_read_only && !terminalStatuses.has(taskStatus(task)),
    )
  )
    return null;
  const source = detail.tasks.find(
    (task) => !task.source_read_only && task.run_id,
  );
  const run = source?.snapshot?.run;
  return source &&
    run &&
    source.project_id === thread.project_id &&
    source.node_id === thread.node_id &&
    source.subject === identity &&
    source.thread_id === thread.thread_id &&
    source.run_id === run.run_id &&
    run.session_id === thread.session_id &&
    !run.archived &&
    terminalStatuses.has(run.status)
    ? source
    : null;
}

export function chatTitleError(value: string) {
  if (!value.trim()) return "Enter a chat title.";
  if (new TextEncoder().encode(value.trim()).length > 256)
    return "Use a title of 256 UTF-8 bytes or fewer.";
  if (/[\u0000-\u001f\u007f]/u.test(value.trim()))
    return "Remove control characters from the title.";
  return "";
}
