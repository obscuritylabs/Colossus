import { Metadata, type ServiceError } from "@grpc/grpc-js";
import type {
  AgentCommunicationServiceClient,
  AgentMessage,
  AgentParticipant,
  AgentTaskSnapshot,
  SendAgentMessageRequest,
  SubmitAgentTaskMessageRequest,
  GetAgentTaskRequest,
  ListAgentTasksRequest,
  ListAgentTasksResponse,
  ListAgentMessagesResponse,
  WatchAgentMessagesResponse,
} from "./gen/colossus/api/v1alpha1/communication.js";

const invalid = () =>
  new TypeError("Invalid bounded agent communication record");
function token(value: string) {
  if (!/^[A-Za-z0-9._:-]{1,128}$/u.test(value)) throw invalid();
}
export function validateAgentMessage(
  message: AgentMessage | undefined,
): AgentMessage {
  if (!message) throw invalid();
  for (const id of [message.id, message.rootRunId, message.recipientId])
    token(id);
  if (
    !message.sender ||
    !message.receipt ||
    message.sequence <= 0n ||
    message.sequence > 4096n ||
    !message.text ||
    new TextEncoder().encode(message.text).length > 16 * 1024
  )
    throw invalid();
  token(message.sender.value);
  const receipt = message.receipt;
  switch (receipt.state) {
    case "accepted":
      if (
        receipt.runId !== undefined ||
        receipt.turn !== undefined ||
        receipt.requestHash !== undefined ||
        receipt.reason !== undefined
      )
        throw invalid();
      break;
    case "included_in_turn":
      if (
        !receipt.runId ||
        !receipt.turn ||
        receipt.turn > 65535 ||
        !receipt.requestHash ||
        !/^[0-9a-f]{64}$/u.test(receipt.requestHash) ||
        receipt.reason !== undefined
      )
        throw invalid();
      token(receipt.runId);
      break;
    case "not_delivered":
      if (
        ![
          "completed",
          "cancelled",
          "failed",
          "interrupted",
          "budget_exhausted",
          "superseded",
        ].includes(receipt.reason ?? "") ||
        receipt.runId !== undefined ||
        receipt.turn !== undefined ||
        receipt.requestHash !== undefined
      )
        throw invalid();
      break;
    default:
      throw invalid();
  }
  return message;
}
function task(task: AgentTaskSnapshot | undefined): AgentTaskSnapshot {
  if (
    !task ||
    task.history.length > 16 ||
    ![
      "queued",
      "running",
      "waiting",
      "cancelling",
      "completed",
      "failed",
      "cancelled",
      "interrupted",
      "outcome_unknown",
    ].includes(task.status)
  )
    throw invalid();
  token(task.taskId);
  token(task.contextId);
  for (const input of task.history)
    if (
      input.taskId !== task.taskId ||
      input.contextId !== task.contextId ||
      new TextEncoder().encode(input.text).length > 16 * 1024
    )
      throw invalid();
  return task;
}
function unary<T>(
  invoke: (
    callback: (error: ServiceError | null, value?: T) => void,
  ) => unknown,
): Promise<T> {
  return new Promise((resolve, reject) =>
    invoke((error, value) => {
      if (error) reject(error);
      else if (value === undefined) reject(invalid());
      else resolve(value);
    }),
  );
}
/** Optional caller-bound communication over an already authenticated SDK channel. No mutation retries. */
export class AgentCommunication {
  constructor(private readonly client: AgentCommunicationServiceClient) {}
  async participants(rootRunId: string): Promise<AgentParticipant[]> {
    token(rootRunId);
    const response = await unary<{ participants: AgentParticipant[] }>(
      (callback) =>
        this.client.listAgentParticipants(
          { rootRunId },
          new Metadata(),
          { deadline: Date.now() + 30_000 },
          callback,
        ),
    );
    if (response.participants.length > 128) throw invalid();
    for (const participant of response.participants) {
      token(participant.id);
      if (
        participant.rootRunId !== rootRunId ||
        participant.generation <= 0n ||
        participant.generation > 128n ||
        participant.pendingMessages > 64 ||
        participant.pendingBytes > 256 * 1024 ||
        participant.open === (participant.closedReason !== undefined)
      )
        throw invalid();
    }
    return response.participants;
  }
  async send(request: SendAgentMessageRequest): Promise<AgentMessage> {
    token(request.recipientId);
    token(request.idempotencyKey);
    if (
      !request.text ||
      new TextEncoder().encode(request.text).length > 16 * 1024
    )
      throw invalid();
    const response = await unary<{ message: AgentMessage | undefined }>(
      (callback) =>
        this.client.sendAgentMessage(
          request,
          new Metadata(),
          { deadline: Date.now() + 30_000 },
          callback,
        ),
    );
    const message = validateAgentMessage(response.message);
    if (message.recipientId !== request.recipientId) throw invalid();
    return message;
  }
  async get(messageId: string): Promise<AgentMessage> {
    token(messageId);
    const response = await unary<{ message: AgentMessage | undefined }>(
      (callback) =>
        this.client.getAgentMessage(
          { messageId },
          new Metadata(),
          { deadline: Date.now() + 30_000 },
          callback,
        ),
    );
    const message = validateAgentMessage(response.message);
    if (message.id !== messageId) throw invalid();
    return message;
  }
  async messages(
    participantId: string,
    afterSequence = 0n,
    limit = 16,
  ): Promise<ListAgentMessagesResponse> {
    token(participantId);
    if (
      afterSequence < 0n ||
      !Number.isInteger(limit) ||
      limit < 1 ||
      limit > 16
    )
      throw invalid();
    const page = await unary<ListAgentMessagesResponse>((callback) =>
      this.client.listAgentMessages(
        { participantId, afterSequence, limit },
        new Metadata(),
        { deadline: Date.now() + 30_000 },
        callback,
      ),
    );
    if (page.messages.length > limit) throw invalid();
    let cursor = afterSequence;
    for (const value of page.messages) {
      const message = validateAgentMessage(value);
      if (
        message.recipientId !== participantId ||
        message.sequence !== ++cursor
      )
        throw invalid();
    }
    if (
      page.nextSequence !== cursor ||
      (page.hasMore && page.messages.length === 0)
    )
      throw invalid();
    return page;
  }
  async *watch(
    rootRunId: string,
    afterSequence = 0n,
  ): AsyncIterable<WatchAgentMessagesResponse> {
    token(rootRunId);
    let cursor = afterSequence;
    const call = this.client.watchAgentMessages({ rootRunId, afterSequence });
    try {
      for await (const value of call) {
        const update = value as WatchAgentMessagesResponse;
        if (
          update.sequence !== ++cursor ||
          validateAgentMessage(update.message).rootRunId !== rootRunId
        )
          throw invalid();
        yield update;
      }
    } finally {
      call.cancel();
    }
  }
  async submitTaskMessage(
    request: SubmitAgentTaskMessageRequest,
  ): Promise<AgentTaskSnapshot> {
    const response = await unary<{ task: AgentTaskSnapshot | undefined }>(
      (callback) =>
        this.client.submitAgentTaskMessage(
          request,
          new Metadata(),
          { deadline: Date.now() + 30_000 },
          callback,
        ),
    );
    return task(response.task);
  }
  async getTask(request: GetAgentTaskRequest): Promise<AgentTaskSnapshot> {
    const response = await unary<{ task: AgentTaskSnapshot | undefined }>(
      (callback) =>
        this.client.getAgentTask(
          request,
          new Metadata(),
          { deadline: Date.now() + 30_000 },
          callback,
        ),
    );
    const value = task(response.task);
    if (
      value.taskId !== request.taskId ||
      value.history.length > request.historyLength
    )
      throw invalid();
    return value;
  }
  async listTasks(
    request: ListAgentTasksRequest,
  ): Promise<ListAgentTasksResponse> {
    const response = await unary<ListAgentTasksResponse>((callback) =>
      this.client.listAgentTasks(
        request,
        new Metadata(),
        { deadline: Date.now() + 30_000 },
        callback,
      ),
    );
    if (response.tasks.length > request.pageSize) throw invalid();
    response.tasks.forEach(task);
    return response;
  }
}
