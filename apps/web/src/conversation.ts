import {
  taskStatus,
  visibleOutput,
  type Task,
  type ThreadDetailResponse,
  type ThreadMessage,
  type Update,
} from "./api";

export interface ConversationMessage extends ThreadMessage {
  status?: string;
}
export interface ConversationTurn {
  key: string;
  task: Task | undefined;
  messages: ConversationMessage[];
  updates: Update[];
}
const createdTime = (task: Task) =>
  task.created_at || task.snapshot?.run.created_at || "";

/** Merge released messages and compatible run feeds without reordering completed turns. */
export function conversationProjection(
  detail: ThreadDetailResponse,
  events: Update[],
) {
  const byTask = new Map<string, Update[]>(),
    tasks = new Map(detail.tasks.map((task) => [task.task_id, task])),
    tasksByRun = new Map<string, Task | null>(),
    seen = new Set<string>();
  for (const task of detail.tasks) {
    byTask.set(task.task_id, []);
    if (task.run_id)
      tasksByRun.set(task.run_id, tasksByRun.has(task.run_id) ? null : task);
  }
  for (const event of events) {
    const task = event.task_id
      ? tasks.get(event.task_id)
      : tasksByRun.get(event.run_id);
    // A task claim cannot redirect another run's output or delegated activity.
    if (
      !task ||
      !task.run_id ||
      event.run_id !== task.run_id ||
      !Number.isSafeInteger(event.sequence) ||
      event.sequence <= 0
    )
      continue;
    const key = `${event.run_id}:${event.sequence}`;
    if (seen.has(key)) continue;
    seen.add(key);
    byTask.get(task.task_id)!.push(event);
  }
  for (const updates of byTask.values())
    updates.sort((a, b) => a.sequence - b.sequence);
  const messages: ConversationMessage[] = [...detail.messages];
  const taskTimes = new Map(
    detail.tasks.map((task) => [
      task.task_id,
      Date.parse(createdTime(task)) || 0,
    ]),
  );
  for (const task of detail.tasks) {
    const updates = byTask.get(task.task_id) ?? [];
    if (
      !task.source_read_only &&
      task.subject !== "runtime" &&
      !messages.some(
        (message) =>
          message.task_id === task.task_id && message.role === "user",
      )
    ) {
      const text = task.request.input
        .map((part) => part.text)
        .filter(Boolean)
        .join("\n");
      if (text)
        messages.push({
          message_id: `task-input-${task.task_id}`,
          role: "user",
          text,
          created_at: createdTime(task) || detail.thread.created_at,
          task_id: task.task_id,
        });
    }
    const output = visibleOutput(updates);
    if (
      output &&
      !messages.some(
        (message) =>
          message.task_id === task.task_id && message.role === "assistant",
      )
    ) {
      messages.push({
        message_id: `run-output-${task.task_id}`,
        role: "assistant",
        text: output,
        created_at:
          updates.at(-1)?.created_at ||
          task.snapshot?.run.updated_at ||
          createdTime(task) ||
          detail.thread.updated_at,
        task_id: task.task_id,
        status: taskStatus(task),
      });
    }
  }
  const time = (message: ConversationMessage) =>
    Date.parse(message.created_at) || 0;
  messages.sort(
    (a, b) =>
      (taskTimes.get(a.task_id ?? "") ?? time(a)) -
        (taskTimes.get(b.task_id ?? "") ?? time(b)) ||
      time(a) - time(b) ||
      (a.role === "user" ? 0 : 1) - (b.role === "user" ? 0 : 1) ||
      a.message_id.localeCompare(b.message_id),
  );
  const turns = new Map<string, ConversationTurn>();
  for (const message of messages) {
    const key = message.task_id
      ? `task-${message.task_id}`
      : `message-${message.message_id}`;
    const turn = turns.get(key) ?? {
      key,
      task: message.task_id ? tasks.get(message.task_id) : undefined,
      messages: [],
      updates: message.task_id ? (byTask.get(message.task_id) ?? []) : [],
    };
    turn.messages.push(message);
    turns.set(key, turn);
  }
  for (const task of detail.tasks) {
    const key = `task-${task.task_id}`;
    if (!turns.has(key))
      turns.set(key, {
        key,
        task,
        messages: [],
        updates: byTask.get(task.task_id) ?? [],
      });
  }
  // Timestamps order complete groups; identity alone binds messages and activity.
  const position = (turn: ConversationTurn) =>
    (turn.task ? taskTimes.get(turn.task.task_id) : 0) ||
    (turn.messages[0] ? time(turn.messages[0]) : 0);
  const orderedTurns = [...turns.values()].sort(
    (a, b) => position(a) - position(b) || a.key.localeCompare(b.key),
  );
  return { messages, byTask, turns: orderedTurns };
}
