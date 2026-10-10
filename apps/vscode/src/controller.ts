import { randomUUID } from "node:crypto";
import {
  CreateRunRequest,
  InteractionStatus,
  ResearchDepth,
  ResearchSourceKind,
  RunMode,
  runStatusToJSON,
  type Interaction,
  type RespondInteractionRequest,
  type Run,
  type RunUpdate,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";

import { safeError, type WorkerClient } from "./connection.js";
import {
  initialView,
  isResearchOptions,
  isWorkMode,
  supportsResearch,
  type ContextView,
  type ResearchOptions,
  type WorkMode,
  type WorkView,
} from "./model.js";
import { UserError } from "./errors.js";
import { toolProgress } from "./tool-progress.js";
import {
  runView,
  planView,
  plansFor,
  planOutputRun,
  activityView,
} from "./state.js";

export interface ControllerHost {
  publish(view: WorkView): void;
  remember(sessionId: string): Promise<void>;
  interaction(
    interaction: Interaction,
  ): Promise<RespondInteractionRequest["response"]>;
}

export class WorkController {
  readonly view: WorkView;
  private client: WorkerClient | undefined;
  private role = "primary";
  private epoch = 0;
  private active: Run | undefined;
  private abort: AbortController | undefined;
  private historyAbort: AbortController | undefined;
  private toolHistory = new Map<string, string>();
  private interactions = new Map<string, Interaction>();
  private responding = new Set<string>();
  private uncertain = false;
  private cancelling = false;
  private attachmentId = 0;
  get connectionGeneration() {
    return this.attachmentId;
  }
  get canReconnect() {
    return (
      this.responding.size === 0 &&
      !this.cancelling &&
      (!this.view.busy ||
        (!this.view.watching && (!!this.active || this.uncertain)))
    );
  }
  private knownRuns = new Map<string, Run>();
  private historyCursor = "";
  private historyRevision = 0;
  private inspectionRevision = 0;

  constructor(
    workspace: string,
    private readonly host: ControllerHost,
  ) {
    this.view = initialView(workspace);
  }
  publish() {
    this.view.reconnectable = this.canReconnect;
    this.host.publish(structuredClone(this.view));
  }
  error(error: unknown) {
    this.view.error = safeError(error);
    this.publish();
  }

  async attach(client: WorkerClient, role: string, sessionId = "") {
    this.detach();
    this.client = client;
    this.role = role;
    const epoch = this.epoch;
    this.view.connected = true;
    this.view.connecting = false;
    this.view.version = client.info.serverVersion;
    this.view.capabilities = client.info.capabilities.map(
      ({ name, enabled, detail }) => ({ name, enabled, detail }),
    );
    this.view.error = "";
    this.view.status = "Ready";
    this.publish();
    await this.refreshSessions();
    if (epoch !== this.epoch) return;
    if (sessionId) await this.selectSession(sessionId, true);
  }

  detach() {
    this.attachmentId++;
    this.epoch++;
    this.abort?.abort();
    this.historyAbort?.abort();
    this.toolHistory.clear();
    this.abort = undefined;
    this.client?.close();
    this.client = undefined;
    this.active = undefined;
    this.knownRuns.clear();
    this.historyCursor = "";
    this.historyRevision++;
    this.inspectionRevision++;
    this.view.runs = [];
    this.view.plans = [];
    this.view.sessions = [];
    this.view.capabilities = [];
    this.view.inspection = undefined;
    this.view.inspectionLoading = false;
    this.view.historyHasMore = false;
    this.view.historyLoading = false;
    this.interactions.clear();
    this.responding.clear();
    this.uncertain = false;
    this.view.connected = false;
    this.view.connecting = false;
    this.view.busy = false;
    this.view.watching = false;
    this.view.interactions = [];
    this.view.status = "Disconnected";
    this.publish();
  }

  async refreshSessions(more = false) {
    const client = this.requireClient();
    if (more && (!this.historyCursor || this.view.historyLoading)) return;
    const epoch = this.epoch;
    const revision = ++this.historyRevision;
    this.view.historyLoading = true;
    this.publish();
    try {
      const response = await client.listRuns(
        "",
        more ? this.historyCursor : "",
      );
      if (epoch !== this.epoch || revision !== this.historyRevision) return;
      if (!more) {
        const selected = [...this.knownRuns.values()].filter(
          (run) => run.sessionId === this.view.sessionId,
        );
        this.knownRuns.clear();
        for (const run of selected) this.knownRuns.set(run.runId, run);
      }
      for (const run of response.runs) this.knownRuns.set(run.runId, run);
      this.historyCursor = response.page?.nextPageToken ?? "";
      this.view.historyHasMore = !!this.historyCursor;
      this.syncState();
    } finally {
      if (client === this.client && revision === this.historyRevision) {
        this.view.historyLoading = false;
        this.publish();
      }
    }
  }

  private orderedRuns() {
    return [...this.knownRuns.values()].sort(
      (a, b) =>
        (b.createdAt?.getTime() ?? 0) - (a.createdAt?.getTime() ?? 0) ||
        b.runId.localeCompare(a.runId),
    );
  }
  private syncState() {
    const runs = this.orderedRuns();
    this.view.runs = runs.map(runView);
    this.view.plans = plansFor(runs);
    const sessions = new Map<string, WorkView["sessions"][number]>();
    for (const run of runs) {
      if (!sessions.has(run.sessionId))
        sessions.set(run.sessionId, {
          id: run.sessionId,
          title: run.title || "Untitled conversation",
          status: runView(run).status,
          updatedAt: runView(run).updatedAt,
        });
    }
    this.view.sessions = [...sessions.values()];
  }
  private recordRun(run: Run | undefined) {
    if (!run) return;
    this.knownRuns.set(run.runId, run);
    this.syncState();
  }

  async inspect(id: string, kind: "run" | "plan") {
    const client = this.requireClient();
    const selectedPlan =
      kind === "plan" ? this.view.plans.find((p) => p.id === id) : undefined;
    const bound = this.knownRuns.get(
      kind === "plan" ? (selectedPlan?.sourceRunId ?? "") : id,
    );
    if (!bound) throw new UserError("Refresh and select a listed run or plan.");
    const revision = ++this.inspectionRevision;
    this.view.inspection = undefined;
    this.view.inspectionLoading = true;
    this.view.error = "";
    this.publish();
    const current = () =>
      revision === this.inspectionRevision && client === this.client;
    try {
      const response = await client.get(bound.runId);
      if (!current()) return;
      const run = response.run;
      if (
        !run ||
        run.runId !== bound.runId ||
        run.sessionId !== bound.sessionId
      )
        throw new UserError("The selected run is no longer available.");
      this.recordRun(run);
      const plan = planView(run);
      if (kind === "plan" && (!plan || plan.id !== id))
        throw new UserError(
          "The selected plan is no longer available. Refresh the workspace.",
        );
      let output =
        run.terminal?.$case === "result"
          ? run.terminal.value.output
          : (run.terminal?.value.message ?? "");
      if (kind === "plan" && plan) {
        const source = planOutputRun(this.orderedRuns(), plan);
        if (source && source.runId !== run.runId) {
          const original = await client.get(source.runId);
          if (!current()) return;
          if (
            original.run?.runId === source.runId &&
            original.run.sessionId === run.sessionId &&
            planView(original.run)?.id === plan.id &&
            planView(original.run)?.revision === plan.revision &&
            original.run.terminal?.$case === "result"
          )
            output = original.run.terminal.value.output;
        }
      }
      let activities: import("./model.js").ActivityView[] = [];
      let activityState =
        "Session activity is unavailable for this worker or enrollment.";
      let activityHasMore = false;
      if (
        client.info.capabilities.some(
          (c) => c.name === "sessions.activity" && c.enabled,
        )
      ) {
        try {
          const activity = await client.activity(run.runId);
          if (!current()) return;
          activities = activity.activities.map(activityView);
          activityHasMore = !!activity.page?.nextPageToken;
          activityState = activity.caughtUp
            ? "Projection is up to date."
            : "Projection is catching up.";
        } catch (error) {
          if (!current()) return;
          activityState = `Activity could not be loaded. ${safeError(error)}`;
        }
      }
      if (!current()) return;
      this.view.inspection = {
        agentInboxesAvailable: client.info.capabilities.some(
          (capability) =>
            capability.name === "agent_messages.read.v1" && capability.enabled,
        ),
        run: runView(run),
        plan,
        output,
        model:
          run.terminal?.$case === "result"
            ? run.terminal.value.modelProfile || run.terminal.value.model
            : "",
        provider:
          run.terminal?.$case === "result"
            ? run.terminal.value.providerProfile
            : "",
        activities,
        activityState,
        activityHasMore,
        observedAt: new Date().toISOString(),
      };
    } finally {
      if (current()) {
        this.view.inspectionLoading = false;
        this.publish();
      }
    }
  }

  async inspectAgentInbox(participantId: string | null, afterSequence: number) {
    const client = this.requireClient();
    const source = this.view.inspection;
    if (!source)
      throw new UserError("Select a listed run to inspect its inboxes.");
    const participants = await client.inboxParticipants(source.run.id);
    const records = participants.map((item) => ({
      id: item.id,
      root_run_id: item.rootRunId,
      session_id: item.sessionId,
      run_id: item.runId ?? null,
      parent_id: item.parentId ?? null,
      subagent_id: item.subagentId ?? null,
      generation: Number(item.generation),
      open: item.open,
      closed_reason: item.closedReason ?? null,
      pending_messages: item.pendingMessages,
      pending_bytes: item.pendingBytes,
      created_at: item.createdAt,
    }));
    if (participantId === null) return { participants: records, page: null };
    if (!participants.some((item) => item.id === participantId))
      throw new UserError("The inbox is outside this selected run.");
    const page = await client.inboxMessages(
      participantId,
      BigInt(afterSequence),
    );
    return {
      participants: records,
      page: {
        messages: page.messages.map((item) => ({
          id: item.id,
          root_run_id: item.rootRunId,
          sender:
            item.sender?.$case === "senderParticipantId"
              ? { kind: "participant", participant_id: item.sender.value }
              : {
                  kind: "application",
                  application_id: item.sender?.value ?? "",
                },
          recipient_id: item.recipientId,
          sequence: Number(item.sequence),
          text: item.text,
          reply_to: item.replyTo ?? null,
          accepted_at: item.acceptedAt,
          receipt:
            item.receipt?.state === "included_in_turn"
              ? {
                  state: "included_in_turn",
                  run_id: item.receipt.runId,
                  turn: item.receipt.turn,
                  request_hash: item.receipt.requestHash,
                }
              : item.receipt?.state === "not_delivered"
                ? { state: "not_delivered", reason: item.receipt.reason }
                : { state: "accepted" },
        })),
        next_sequence: Number(page.nextSequence),
        has_more: page.hasMore,
      },
    };
  }

  async newSession() {
    if (this.view.busy)
      throw new UserError("Stop or reconcile the active run first.");
    this.epoch++;
    this.abort?.abort();
    this.historyAbort?.abort();
    this.toolHistory.clear();
    this.active = undefined;
    this.interactions.clear();
    this.view.sessionId = "";
    this.view.messages = [];
    this.view.tools = [];
    this.view.interactions = [];
    this.view.error = "";
    this.view.status = "New conversation";
    await this.host.remember("");
    this.publish();
  }

  async selectSession(id: string, restoring = false) {
    if (this.view.busy)
      throw new UserError(
        "Stop the active run before switching conversations.",
      );
    if (!restoring && !this.view.sessions.some((s) => s.id === id))
      throw new UserError("Select a listed conversation.");
    const client = this.requireClient();
    const epoch = ++this.epoch;
    this.abort?.abort();
    this.historyAbort?.abort();
    this.toolHistory.clear();
    this.interactions.clear();
    this.view.interactions = [];
    this.view.busy = true;
    this.view.status = "Loading conversation";
    this.view.error = "";
    this.publish();
    try {
      const listed = await this.listSessionRuns(client, id, epoch);
      if (!listed) return;
      this.active = listed.find((run) => run.terminal === undefined);
      if (this.active)
        this.view.mode =
          this.active.mode === RunMode.RUN_MODE_RESEARCH
            ? "research"
            : this.active.mode === RunMode.RUN_MODE_PLAN
              ? "plan"
              : "execute";
      this.view.sessionId = id;
      this.view.tools = [];
      for (const run of listed) this.recordRun(run);
      await this.loadMessages(client, epoch, listed);
      if (epoch !== this.epoch) return;
      await this.host.remember(id);
      this.view.busy = this.active !== undefined;
      this.view.status = this.active ? "Reconnecting to active run" : "Ready";
      if (this.active) this.startWatch(this.active.runId, epoch);
      this.publish();
    } catch (error) {
      if (epoch === this.epoch) {
        this.view.busy = false;
        this.error(error);
      }
    }
  }

  setContext(context: ContextView) {
    if (this.view.busy)
      throw new UserError("Wait for the current run before changing context.");
    const next = [
      ...this.view.context.filter((c) => c.label !== context.label),
      context,
    ];
    if (
      next.length > 8 ||
      Buffer.byteLength(next.map((c) => c.text).join("\n"), "utf8") > 96 * 1024
    )
      throw new UserError(
        "Editor context is limited to eight excerpts and 96 KiB.",
      );
    this.view.context = next;
    this.publish();
  }
  clearContext() {
    if (!this.view.busy) {
      this.view.context = [];
      this.publish();
    }
  }

  async send(text: string, mode: WorkMode, research?: ResearchOptions) {
    if (this.view.busy || this.uncertain)
      throw new UserError(
        "Reconcile the current run before sending another task.",
      );
    const client = this.requireClient();
    if (!isWorkMode(mode)) throw new UserError("Choose a supported run mode.");
    if (mode === "research") {
      if (!supportsResearch(client.info.capabilities))
        throw new UserError(
          "Research is unavailable for this worker connection.",
        );
      if (!isResearchOptions(research))
        throw new UserError(
          "Choose a Research depth and at least one unique evidence source.",
        );
    } else if (research !== undefined) {
      throw new UserError("Research settings apply only in Research mode.");
    }
    const prompt = text.trim();
    if (!prompt || Buffer.byteLength(prompt, "utf8") > 64 * 1024)
      throw new UserError("Enter a task of at most 64 KiB.");
    const epoch = ++this.epoch;
    this.abort?.abort();
    this.historyAbort?.abort();
    this.view.watching = false;
    this.view.busy = true;
    this.view.mode = mode;
    this.view.error = "";
    this.view.status = "Starting run";
    const input = [
      prompt,
      ...this.view.context.map(
        (c) => `User-supplied editor snapshot: ${c.label}\n${c.text}`,
      ),
    ].join("\n\n");
    this.publish();
    let created = false;
    try {
      const response = await client.create(
        CreateRunRequest.fromPartial({
          input: [{ content: { $case: "text", value: { text: input } } }],
          sessionId: this.view.sessionId || undefined,
          role: this.role,
          mode:
            mode === "research"
              ? RunMode.RUN_MODE_RESEARCH
              : mode === "plan"
                ? RunMode.RUN_MODE_PLAN
                : RunMode.RUN_MODE_EXECUTE,
          ...(mode === "research" && research
            ? {
                researchDepth: {
                  quick: ResearchDepth.RESEARCH_DEPTH_QUICK,
                  standard: ResearchDepth.RESEARCH_DEPTH_STANDARD,
                  deep: ResearchDepth.RESEARCH_DEPTH_DEEP,
                }[research.researchDepth],
                researchSources: research.researchSources.map(
                  (source) =>
                    ({
                      repo: ResearchSourceKind.RESEARCH_SOURCE_KIND_REPO,
                      web: ResearchSourceKind.RESEARCH_SOURCE_KIND_WEB,
                      mcp: ResearchSourceKind.RESEARCH_SOURCE_KIND_MCP,
                    })[source],
                ),
              }
            : {}),
          idempotencyKey: randomUUID(),
        }),
      );
      if (epoch !== this.epoch) return;
      if (!response.run?.runId || !response.run.sessionId)
        throw new UserError("CreateRun returned no durable identity.");
      created = true;
      this.active = response.run;
      this.recordRun(response.run);
      this.view.sessionId = response.run.sessionId;
      this.view.messages.push({
        id: `user:${response.run.runId}`,
        runId: response.run.runId,
        role: "user",
        text: input,
      });
      this.view.context = [];
      await this.host.remember(this.view.sessionId);
      this.startWatch(response.run.runId, epoch);
      this.publish();
      void this.refreshSessions().catch((error) => {
        if (epoch === this.epoch) this.error(error);
      });
    } catch (error) {
      if (epoch !== this.epoch) return;
      this.uncertain = !created;
      this.view.busy = !created;
      this.view.status = created
        ? "Run created. Reconnect to observe it."
        : "Run creation could not be confirmed. Reconnect and inspect recent conversations before starting another task.";
      this.error(error);
    }
  }

  async stop() {
    const client = this.requireClient();
    const run = this.active;
    const epoch = this.epoch;
    if (!run)
      throw new UserError("No known active run. Reconnect to reconcile it.");
    this.cancelling = true;
    this.view.status = "Requesting cancellation";
    this.publish();
    // A cancellation is sent once. The watch remains open until durable terminal evidence.
    try {
      await client.cancel(run.runId, randomUUID());
      if (epoch === this.epoch) this.view.status = "Cancellation requested";
    } finally {
      this.cancelling = false;
      if (epoch === this.epoch) this.publish();
    }
  }

  async resume() {
    const client = this.requireClient();
    const run = this.active;
    if (!run || this.view.watching) return;
    const epoch = this.epoch;
    const response = await client.get(run.runId);
    if (epoch !== this.epoch) return;
    this.interactions.clear();
    for (const interaction of response.pendingInteractions)
      this.addInteraction(interaction);
    this.view.messages = this.view.messages.filter(
      (m) => m.id !== `assistant:${run.runId}`,
    );
    this.view.tools = this.view.tools.filter(
      (tool) => tool.runId !== run.runId,
    );
    this.startWatch(run.runId, epoch);
    this.publish();
  }

  async respond(id: string) {
    if (this.responding.has(id)) return;
    const client = this.requireClient();
    const run = this.active;
    const epoch = this.epoch;
    if (!run || !this.interactions.has(id))
      throw new UserError("Interaction is no longer pending.");
    this.responding.add(id);
    this.publish();
    try {
      // Reconcile the exact caller-owned pending obligation before presenting native UI.
      const current = await client.get(run.runId);
      if (epoch !== this.epoch) return;
      const interaction = current.pendingInteractions.find(
        (i) =>
          i.interactionId === id &&
          i.respondableByCaller &&
          i.status === InteractionStatus.INTERACTION_STATUS_PENDING,
      );
      if (!interaction) {
        this.interactions.delete(id);
        this.syncInteractions();
        return;
      }
      const response = await this.host.interaction(interaction);
      if (!response || epoch !== this.epoch || this.active?.runId !== run.runId)
        return;
      await client.respond({
        runId: run.runId,
        interactionId: interaction.interactionId,
        etag: interaction.etag,
        idempotencyKey: randomUUID(),
        response,
      });
      if (epoch === this.epoch) {
        this.interactions.delete(id);
        this.syncInteractions();
      }
    } catch (error) {
      if (epoch === this.epoch) {
        // An uncertain response is reconciled by GetRun before another user attempt.
        const current = await client.get(run.runId).catch(() => undefined);
        if (epoch !== this.epoch) return;
        if (!current) {
          this.abort?.abort();
          this.view.watching = false;
          this.view.status =
            "Response outcome could not be confirmed. Reconnect the worker to reconcile it.";
        }
        this.interactions.clear();
        for (const interaction of current?.pendingInteractions ?? [])
          this.addInteraction(interaction);
        this.syncInteractions();
        this.error(error);
      }
    } finally {
      this.responding.delete(id);
      if (epoch === this.epoch) this.publish();
    }
  }

  private startWatch(id: string, epoch: number) {
    this.abort?.abort();
    const abort = new AbortController();
    this.abort = abort;
    const client = this.requireClient();
    this.view.watching = true;
    this.view.status = "Working";
    void (async () => {
      try {
        // GetRun restores obligations even when their original event predates this view.
        const current = await client.get(id);
        if (epoch !== this.epoch || abort.signal.aborted) return;
        this.recordRun(current.run);
        for (const interaction of current.pendingInteractions)
          this.addInteraction(interaction);
        this.publish();
        for await (const item of client.watch(id, abort.signal)) {
          if (epoch !== this.epoch || abort.signal.aborted) return;
          this.applyUpdate(item.value);
          this.publish();
        }
        if (epoch !== this.epoch || abort.signal.aborted) return;
        this.view.watching = false;
        const reconciled = await client.get(id);
        if (epoch !== this.epoch || abort.signal.aborted) return;
        this.recordRun(reconciled.run);
        if (reconciled.run?.terminal) {
          this.toolHistory.set(id, "");
          this.active = undefined;
          this.view.busy = false;
          this.interactions.clear();
          this.syncInteractions();
          await this.loadMessages(client, epoch);
          if (epoch !== this.epoch) return;
          await this.refreshSessions();
        }
        this.publish();
      } catch (error) {
        if (epoch === this.epoch && !abort.signal.aborted) {
          this.view.watching = false;
          this.view.status =
            "Observation paused. Reconnect the worker to reconcile the run.";
          this.error(error);
        }
      }
    })();
  }

  private applyUpdate(item: RunUpdate) {
    const recorded = this.knownRuns.get(item.runId);
    if (recorded) {
      recorded.lastSequence = item.sequence;
      if (item.update?.$case === "state")
        recorded.status = item.update.value.status;
      this.syncState();
    }
    const update = item.update;
    switch (update?.$case) {
      case "outputDelta": {
        const id = `assistant:${item.runId}`;
        let message = this.view.messages.find((m) => m.id === id);
        if (!message) {
          message = { id, runId: item.runId, role: "assistant", text: "" };
          this.view.messages.push(message);
        }
        message.text = (message.text + update.value.text).slice(-512 * 1024);
        break;
      }
      case "toolActivity": {
        this.view.tools = toolProgress(this.view.tools, item);
        break;
      }
      case "interaction":
        this.addInteraction(update.value);
        break;
      case "state":
        this.view.status = runStatusToJSON(update.value.status)
          .replace("RUN_STATUS_", "")
          .toLowerCase();
        break;
      case "notice":
        this.view.status = update.value.message;
        break;
      case "providerRetry":
        this.view.status = `Provider recovery: attempt ${update.value.attempt} of ${update.value.maxRetries}`;
        break;
      case "result": {
        const id = `assistant:${item.runId}`;
        const existing = this.view.messages.find((m) => m.id === id);
        if (existing) existing.text = update.value.output;
        else
          this.view.messages.push({
            id,
            runId: item.runId,
            role: "assistant",
            text: update.value.output,
          });
        this.view.status = "Completed";
        break;
      }
      case "failure":
        this.view.error = update.value.failure?.message || "Run failed";
        this.view.status = "Failed";
        break;
      case "cancellation":
        this.view.status = "Cancelled";
        break;
      default:
        break;
    }
  }

  private addInteraction(interaction: Interaction) {
    if (interaction.status === InteractionStatus.INTERACTION_STATUS_PENDING)
      this.interactions.set(interaction.interactionId, interaction);
    else this.interactions.delete(interaction.interactionId);
    this.syncInteractions();
  }
  private syncInteractions() {
    this.view.interactions = [...this.interactions.values()].map((i) => ({
      id: i.interactionId,
      title:
        i.content?.$case === "approval"
          ? i.content.value.reason
          : i.content?.$case === "userPrompt"
            ? i.content.value.question
            : "Interaction",
      kind: i.content?.$case === "approval" ? "approval" : "question",
      respondable: i.respondableByCaller,
    }));
    this.publish();
  }

  private async listSessionRuns(
    client: WorkerClient,
    sessionId: string,
    epoch: number,
  ): Promise<Run[] | undefined> {
    const runs = new Map<string, Run>();
    const cursors = new Set<string>();
    let cursor = "";
    // The worker caps each page at three runs. Keep the latest 20 turns,
    // with a bounded traversal even if a malformed server repeats a cursor.
    for (let page = 0; page < 20; page++) {
      const response = await client.listRuns(sessionId, cursor);
      if (epoch !== this.epoch) return undefined;
      for (const run of response.runs) {
        if (run.sessionId !== sessionId)
          throw new UserError(
            "Conversation history belongs to another session.",
          );
        runs.set(run.runId, run);
        if (runs.size === 20) return [...runs.values()];
      }
      const next = response.page?.nextPageToken ?? "";
      if (!next) return [...runs.values()];
      if (cursors.has(next) || response.runs.length === 0)
        throw new UserError("Conversation history could not be paginated.");
      cursors.add(next);
      cursor = next;
    }
    throw new UserError("Conversation history exceeded its page limit.");
  }

  private async loadMessages(
    client: WorkerClient,
    epoch: number,
    runs?: Run[],
  ) {
    const sessionId = this.view.sessionId;
    const listed =
      runs ?? (await this.listSessionRuns(client, sessionId, epoch));
    if (!listed || epoch !== this.epoch) return;
    const previous = new Map(
      this.view.messages.map((message) => [message.id, message]),
    );
    const messages: WorkView["messages"] = [];
    const historyDeadline = Date.now() + 5000;
    const recent = listed.slice(0, 20);
    const retained = new Set(recent.map((run) => run.runId));
    this.view.tools = this.view.tools.filter(
      (tool) => tool.runId && retained.has(tool.runId),
    );
    for (const id of this.toolHistory.keys())
      if (!retained.has(id)) this.toolHistory.delete(id);
    const terminals = new Map<string, Run["terminal"]>();
    // Restore the most recent progress first within the read budget, then render
    // chronological turns. A slow old feed cannot starve the latest task.
    for (const summary of recent) {
      if (
        summary.runId === this.active?.runId &&
        summary.terminal === undefined
      )
        continue;
      const response = await client.get(summary.runId);
      if (epoch !== this.epoch) return;
      this.recordRun(response.run);
      terminals.set(summary.runId, response.run?.terminal);
      if (response.run?.terminal) {
        await this.loadToolHistory(
          client,
          summary.runId,
          epoch,
          historyDeadline,
        );
        if (epoch !== this.epoch) return;
      }
    }
    // The current worker serves durable runs. SessionService is a schema-only future surface.
    for (const summary of recent.reverse()) {
      const userId = `user:${summary.runId}`;
      messages.push(
        previous.get(userId) ?? {
          id: userId,
          runId: summary.runId,
          role: "user",
          text: summary.title || "Earlier task",
          summary: true,
        },
      );
      if (
        summary.runId === this.active?.runId &&
        summary.terminal === undefined
      )
        continue;
      const terminal = terminals.get(summary.runId);
      if (terminal) {
        const warning = this.toolHistory.get(summary.runId);
        if (warning)
          messages.push({
            id: `tools:${summary.runId}`,
            runId: summary.runId,
            role: "notice",
            text: warning,
          });
      }
      if (terminal?.$case === "result")
        messages.push({
          id: `assistant:${summary.runId}`,
          runId: summary.runId,
          role: "assistant",
          text: terminal.value.output,
        });
      else if (terminal)
        messages.push({
          id: `notice:${summary.runId}`,
          runId: summary.runId,
          role: "notice",
          text: terminal.value.message,
        });
    }
    this.view.messages = messages;
  }

  private async loadToolHistory(
    client: WorkerClient,
    id: string,
    epoch: number,
    deadline: number,
  ) {
    if (this.toolHistory.has(id)) return;
    const warning =
      "Some earlier tool progress could not be loaded. The saved response is still available.";
    if (Date.now() >= deadline) {
      this.toolHistory.set(id, warning);
      return;
    }
    const abort = new AbortController();
    this.historyAbort = abort;
    const timer = setTimeout(
      () => abort.abort(),
      Math.min(2000, deadline - Date.now()),
    );
    let count = 0;
    let complete = false;
    let tools: WorkView["tools"] = [];
    try {
      // Reading an already terminal run's feed restores released progress. It
      // never creates a run, executes a tool, or responds to an interaction.
      for await (const item of client.watch(id, abort.signal)) {
        if (epoch !== this.epoch || abort.signal.aborted) return;
        tools = toolProgress(tools, item.value);
        const kind = item.value.update?.$case;
        if (kind === "result" || kind === "failure" || kind === "cancellation")
          complete = true;
        if (++count >= 2000 && !complete) break;
      }
    } catch {
      // History is supplementary; a failed read must not hide canonical output.
    } finally {
      clearTimeout(timer);
      abort.abort();
      if (this.historyAbort === abort) this.historyAbort = undefined;
      if (epoch === this.epoch) {
        // Older feeds load after recent feeds. Keep recent calls when the
        // conversation-wide limit is reached, rather than evicting them.
        this.view.tools = [
          ...tools,
          ...this.view.tools.filter((tool) => tool.runId !== id),
        ].slice(-100);
        this.toolHistory.set(id, complete ? "" : warning);
      }
    }
  }

  private requireClient() {
    if (!this.client) throw new UserError("Connect a worker first.");
    return this.client;
  }
}
