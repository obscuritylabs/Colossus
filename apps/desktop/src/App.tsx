import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import type { FormEvent } from "react";

import {
  CommandFailure,
  cancelRun,
  connectColossus,
  createRun,
  getRun,
  listRuns,
  respondInteraction,
  watchRun,
} from "./api";
import type { AgentParticipant, AgentWorkState } from "./components/AgentFlow";
import type {
  ArtifactPreviewLine,
  ArtifactViewItem,
} from "./components/ArtifactWorkspace";
import { ContextSidebar } from "./components/ContextSidebar";
import { OperationsSurface } from "./components/OperationsSurface";
import { ProductRail } from "./components/ProductRail";
import type { WorkspaceSurface } from "./components/ProductRail";
import { WorkComposer } from "./components/WorkComposer";
import { WorkSidebar } from "./components/WorkSidebar";
import { WorkSurface } from "./components/WorkSurface";
import { buildOperationsStudioFixture } from "./fixtures";
import {
  agentRoleLabel,
  selectOperationalActivity,
  selectReleasedArtifacts,
} from "./presenters";
import {
  MAX_PROMPT_BYTES,
  MAX_TURNS,
  chatReducer,
  clampMaxTurns,
  connectionStateForError,
  initialChatState,
  isConnectionError,
  isPromptWithinByteLimit,
  operationFingerprint,
  stableIdempotentAttempt,
  utf8ByteLength,
  withBoundedEntry,
} from "./state";
import type { IdempotentAttempt } from "./state";
import type {
  CommandError,
  ConnectionStatus,
  CreateRunRequest,
  Interaction,
  InteractionAnswer,
  Run,
  RunMode,
  RunStatus,
  WatchEvent,
} from "./types";
import { isTerminalStatus } from "./types";

const FIXTURE_MODE =
  import.meta.env.DEV &&
  new URLSearchParams(window.location.search).get("fixture") ===
    "operations-studio";

const INITIAL_CONNECTION: ConnectionStatus = FIXTURE_MODE
  ? {
      state: "connected",
      message:
        "Development showcase connected to a deterministic local fixture.",
    }
  : {
      state: "disconnected",
      message: "Connecting to the local Colossus agent…",
    };

const FALLBACK_ACTION_ERROR: CommandError = {
  code: "desktop_request_failed",
  message:
    "The request could not be completed. Check the connection and retry.",
  retryable: true,
  outcomeUnknown: false,
  violations: [],
};

const DEMO_PARTICIPANTS: readonly AgentParticipant[] = [
  {
    id: "atlas",
    name: "Atlas",
    role: "Lead",
    state: "coordinating",
    icon: "lead",
  },
  {
    id: "builder",
    name: "Builder",
    role: "Engineer",
    state: "working",
    icon: "builder",
  },
  {
    id: "sentinel",
    name: "Sentinel",
    role: "Security",
    state: "reviewing",
    icon: "security",
  },
  {
    id: "scribe",
    name: "Scribe",
    role: "Writer",
    state: "waiting",
    icon: "writer",
  },
];

const BOOTSTRAP_PREVIEW: readonly ArtifactPreviewLine[] = [
  { number: 74, kind: "context", text: "pub async fn connect_agent(" },
  { number: 75, kind: "context", text: "    config: &DesktopConfig," },
  { number: 76, kind: "context", text: ") -> Result<Client, DesktopError> {" },
  {
    number: 77,
    kind: "deletion",
    text: "    let credential = config.credential.clone();",
  },
  {
    number: 77,
    kind: "addition",
    text: "    let credential = native_keyring::load(",
  },
  { number: 78, kind: "addition", text: "        &config.credential_ref," },
  { number: 79, kind: "addition", text: "    )?;" },
  { number: 80, kind: "addition", text: "" },
  {
    number: 81,
    kind: "addition",
    text: "    let endpoint = config.validated_loopback_endpoint()?;",
  },
  {
    number: 82,
    kind: "addition",
    text: "    let channel = transport::connect(endpoint)",
  },
  { number: 83, kind: "addition", text: "        .await" },
  {
    number: 84,
    kind: "addition",
    text: "        .map_err(DesktopError::connect)?;",
  },
  { number: 85, kind: "context", text: "" },
  {
    number: 86,
    kind: "context",
    text: "    Client::authenticate(channel, credential)",
  },
  { number: 87, kind: "context", text: "        .await" },
  {
    number: 88,
    kind: "addition",
    text: "        .map_err(DesktopError::authenticate)",
  },
  { number: 89, kind: "context", text: "}" },
  { number: 90, kind: "context", text: "" },
  {
    number: 91,
    kind: "addition",
    text: "// Secrets never cross the native command boundary.",
  },
  {
    number: 92,
    kind: "addition",
    text: "// Connection failures remain typed and recoverable.",
  },
];

const TEST_PREVIEW: readonly ArtifactPreviewLine[] = [
  { number: 18, kind: "context", text: "#[tokio::test]" },
  {
    number: 19,
    kind: "addition",
    text: "async fn refuses_non_loopback_agent_endpoints() {",
  },
  {
    number: 20,
    kind: "addition",
    text: '    let error = connect_fixture("https://example.com").await.unwrap_err();',
  },
  {
    number: 21,
    kind: "addition",
    text: '    assert_eq!(error.code(), "desktop_endpoint_rejected");',
  },
  { number: 22, kind: "addition", text: "}" },
];

const NOTES_PREVIEW: readonly ArtifactPreviewLine[] = [
  { number: 1, kind: "context", text: "# Desktop bootstrap hardening" },
  { number: 2, kind: "context", text: "" },
  {
    number: 3,
    kind: "addition",
    text: "- Keep credentials in the native keyring adapter.",
  },
  {
    number: 4,
    kind: "addition",
    text: "- Reject non-loopback endpoints before client creation.",
  },
  {
    number: 5,
    kind: "addition",
    text: "- Return typed, recoverable setup failures to the renderer.",
  },
];

function commandError(error: unknown): CommandError {
  return error instanceof CommandFailure ? error.detail : FALLBACK_ACTION_ERROR;
}

function isCancelable(status: RunStatus): boolean {
  return status === "queued" || status === "running" || status === "waiting";
}

function participantState(status: RunStatus): AgentWorkState {
  if (status === "running" || status === "queued") {
    return "working";
  }
  if (status === "waiting" || status === "cancelling") {
    return "waiting";
  }
  return "idle";
}

function previewFor(
  fileName: string,
): readonly ArtifactPreviewLine[] | undefined {
  if (!FIXTURE_MODE) {
    return undefined;
  }
  if (fileName === "bootstrap.rs") {
    return BOOTSTRAP_PREVIEW;
  }
  if (fileName === "bootstrap.spec.rs") {
    return TEST_PREVIEW;
  }
  if (fileName === "design-notes.md") {
    return NOTES_PREVIEW;
  }
  return undefined;
}

export default function App() {
  const [chat, dispatch] = useReducer(
    chatReducer,
    FIXTURE_MODE ? buildOperationsStudioFixture() : initialChatState,
  );
  const chatRef = useRef(chat);
  const [surface, setSurface] = useState<WorkspaceSurface>("work");
  const [workNavigationOpen, setWorkNavigationOpen] = useState(false);
  const [workQuery, setWorkQuery] = useState("");
  const [connection, setConnection] =
    useState<ConnectionStatus>(INITIAL_CONNECTION);
  const [connecting, setConnecting] = useState(!FIXTURE_MODE);
  const [listBusy, setListBusy] = useState(false);
  const [listError, setListError] = useState("");
  const [runLoadError, setRunLoadError] = useState("");
  const [prompt, setPrompt] = useState("");
  const [role, setRole] = useState("primary");
  const [mode, setMode] = useState<RunMode>("execute");
  const [maxTurns, setMaxTurns] = useState(24);
  const [submitting, setSubmitting] = useState(false);
  const [composerError, setComposerError] = useState<CommandError | null>(null);
  const [actionError, setActionError] = useState<CommandError | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const watchedRuns = useRef(new Map<string, symbol>());
  const connectionGeneration = useRef(0);
  const connectingRef = useRef(false);
  const submitInFlight = useRef(false);
  const createAttempt = useRef<IdempotentAttempt | null>(null);
  const cancelAttempts = useRef(new Map<string, IdempotentAttempt>());
  const responseAttempts = useRef(new Map<string, IdempotentAttempt>());
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const composerFormRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    chatRef.current = chat;
  }, [chat]);

  const markConnectionFailure = useCallback((failure: CommandError) => {
    if (isConnectionError(failure)) {
      setConnection({
        state: connectionStateForError(failure),
        message: failure.message,
      });
    }
  }, []);

  const loadRuns = useCallback(
    async (pageToken: string, append: boolean) => {
      if (FIXTURE_MODE) {
        return true;
      }
      setListBusy(true);
      setListError("");
      try {
        const page = await listRuns({ pageToken });
        dispatch({
          type: append ? "append_recent" : "replace_recent",
          runs: page.runs,
          nextPageToken: page.nextPageToken,
        });
        return true;
      } catch (error: unknown) {
        const failure = commandError(error);
        markConnectionFailure(failure);
        setListError(failure.message);
        return false;
      } finally {
        setListBusy(false);
      }
    },
    [markConnectionFailure],
  );

  const startWatch = useCallback(
    (runId: string, afterSequence: number) => {
      if (FIXTURE_MODE || watchedRuns.current.has(runId)) {
        return;
      }
      const token = Symbol(runId);
      const generation = connectionGeneration.current;
      watchedRuns.current.set(runId, token);
      dispatch({ type: "watch_started", runId });

      const handleEvent = (event: WatchEvent) => {
        if (generation !== connectionGeneration.current) {
          return;
        }
        switch (event.type) {
          case "update":
            dispatch({ type: "ingest_update", update: event.update });
            break;
          case "complete":
            dispatch({ type: "watch_complete", runId: event.runId });
            break;
          case "error":
            markConnectionFailure(event.error);
            dispatch({ type: "watch_error", runId, error: event.error });
            break;
        }
      };

      void watchRun({ runId, afterSequence }, handleEvent)
        .then(() => {
          if (generation === connectionGeneration.current) {
            dispatch({ type: "watch_complete", runId });
          }
        })
        .catch((error: unknown) => {
          if (generation !== connectionGeneration.current) {
            return;
          }
          const failure = commandError(error);
          markConnectionFailure(failure);
          dispatch({ type: "watch_error", runId, error: failure });
        })
        .finally(() => {
          if (watchedRuns.current.get(runId) === token) {
            watchedRuns.current.delete(runId);
          }
        });
    },
    [markConnectionFailure],
  );

  const connect = useCallback(async () => {
    if (FIXTURE_MODE) {
      setConnection(INITIAL_CONNECTION);
      setConnecting(false);
      return;
    }
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      const status = await connectColossus();
      setConnection(status);
      if (status.state === "connected") {
        connectionGeneration.current += 1;
        watchedRuns.current.clear();
        const runsLoaded = await loadRuns("", false);
        if (!runsLoaded) {
          return;
        }
        const activeRunId = chatRef.current.activeRunId;
        const activeView =
          activeRunId === null
            ? undefined
            : chatRef.current.views.get(activeRunId);
        if (
          activeView !== undefined &&
          !isTerminalStatus(activeView.run.status)
        ) {
          startWatch(activeView.run.runId, activeView.lastSequence);
        }
      }
    } catch (error: unknown) {
      const failure = commandError(error);
      setConnection({
        state: connectionStateForError(failure),
        message: failure.message,
      });
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }, [loadRuns, startWatch]);

  useEffect(() => {
    void connect();
  }, [connect]);

  async function openRun(run: Run) {
    if (submitInFlight.current) {
      return;
    }
    setWorkNavigationOpen(false);
    setSurface("work");
    dispatch({ type: "upsert_run", run });
    dispatch({ type: "select_run", runId: run.runId });
    setRunLoadError("");
    setActionError(null);
    if (FIXTURE_MODE) {
      return;
    }
    const existingCursor =
      chatRef.current.views.get(run.runId)?.lastSequence ?? 0;
    try {
      const details = await getRun({ runId: run.runId });
      dispatch({ type: "hydrate_run", details });
      if (!isTerminalStatus(details.run.status)) {
        startWatch(run.runId, existingCursor);
      }
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setRunLoadError(failure.message);
    }
  }

  function newWork() {
    if (submitInFlight.current) {
      return;
    }
    setWorkNavigationOpen(false);
    setSurface("work");
    dispatch({ type: "select_run", runId: null });
    setRunLoadError("");
    setActionError(null);
    setComposerError(null);
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  async function submitRun(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitInFlight.current) {
      return;
    }
    const cleanPrompt = prompt.trim();
    const cleanRole = role.trim();
    if (
      cleanPrompt.length === 0 ||
      cleanRole.length === 0 ||
      connection.state !== "connected" ||
      !isPromptWithinByteLimit(prompt)
    ) {
      return;
    }

    const currentView =
      chat.activeRunId === null ? undefined : chat.views.get(chat.activeRunId);
    const sessionId =
      currentView !== undefined && isTerminalStatus(currentView.run.status)
        ? currentView.run.sessionId
        : undefined;
    const fingerprint = operationFingerprint([
      cleanPrompt,
      sessionId ?? "",
      cleanRole,
      mode,
      maxTurns,
    ]);
    const attempt = stableIdempotentAttempt(createAttempt.current, fingerprint);
    createAttempt.current = attempt;
    const commonRequest: CreateRunRequest = {
      prompt: cleanPrompt,
      role: cleanRole,
      mode,
      maxTurns,
      idempotencyKey: attempt.key,
    };
    const request: CreateRunRequest =
      sessionId === undefined ? commonRequest : { ...commonRequest, sessionId };

    submitInFlight.current = true;
    setSubmitting(true);
    setComposerError(null);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        const now = new Date().toISOString();
        const runId = `fixture-composed-${Date.now()}`;
        const run: Run = {
          runId,
          sessionId: sessionId ?? `fixture-session-${Date.now()}`,
          role: cleanRole,
          mode,
          status: "completed",
          createdAt: now,
          updatedAt: now,
          startedAt: now,
          finishedAt: now,
          lastSequence: 0,
          pendingInteractionCount: 0,
          terminal: {
            type: "result",
            result: {
              output:
                "Showcase response: the request was accepted by the local Operations Studio fixture. Live builds send this through the scoped native command boundary.",
              profile: "desktop-showcase",
              model: "fixture",
              elapsedSeconds: 0.2,
            },
          },
          etag: `fixture-etag-${runId}`,
          selectedSkills: [],
        };
        createAttempt.current = null;
        setPrompt("");
        dispatch({ type: "upsert_run", run });
        dispatch({ type: "record_local_prompt", runId, prompt: cleanPrompt });
        dispatch({ type: "select_run", runId });
        return;
      }

      const run = await createRun(request);
      createAttempt.current = null;
      setPrompt("");
      dispatch({ type: "upsert_run", run });
      dispatch({
        type: "record_local_prompt",
        runId: run.runId,
        prompt: cleanPrompt,
      });
      dispatch({ type: "select_run", runId: run.runId });
      startWatch(run.runId, 0);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setComposerError(failure);
    } finally {
      submitInFlight.current = false;
      setSubmitting(false);
    }
  }

  async function cancelActiveRun() {
    const activeView =
      chat.activeRunId === null ? undefined : chat.views.get(chat.activeRunId);
    if (activeView === undefined || !isCancelable(activeView.run.status)) {
      return;
    }
    const runId = activeView.run.runId;
    if (FIXTURE_MODE) {
      const now = new Date().toISOString();
      dispatch({
        type: "upsert_run",
        run: {
          ...activeView.run,
          status: "cancelled",
          updatedAt: now,
          finishedAt: now,
          pendingInteractionCount: 0,
          terminal: {
            type: "cancellation",
            cancellation: { turn: 1, message: "Stopped in the UI showcase." },
          },
        },
      });
      return;
    }

    const fingerprint = operationFingerprint([runId, "cancel"]);
    const attempt = stableIdempotentAttempt(
      cancelAttempts.current.get(runId) ?? null,
      fingerprint,
    );
    cancelAttempts.current = withBoundedEntry(
      cancelAttempts.current,
      runId,
      attempt,
    );
    setCancelling(true);
    setActionError(null);
    try {
      const run = await cancelRun({ runId, idempotencyKey: attempt.key });
      dispatch({ type: "upsert_run", run });
      startWatch(runId, activeView.lastSequence);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
    } finally {
      setCancelling(false);
    }
  }

  const handleInteraction = useCallback(
    async (interaction: Interaction, response: InteractionAnswer) => {
      if (FIXTURE_MODE) {
        void response;
        dispatch({
          type: "interaction_resolved",
          interaction: { ...interaction, status: "answered" },
        });
        return;
      }
      const fingerprint = operationFingerprint([
        interaction.runId,
        interaction.interactionId,
        interaction.etag,
        response,
      ]);
      const attempt = stableIdempotentAttempt(
        responseAttempts.current.get(interaction.interactionId) ?? null,
        fingerprint,
      );
      responseAttempts.current = withBoundedEntry(
        responseAttempts.current,
        interaction.interactionId,
        attempt,
      );

      try {
        const resolved = await respondInteraction({
          runId: interaction.runId,
          interactionId: interaction.interactionId,
          etag: interaction.etag,
          idempotencyKey: attempt.key,
          response,
        });
        responseAttempts.current.delete(interaction.interactionId);
        dispatch({ type: "interaction_resolved", interaction: resolved });
        const cursor =
          chatRef.current.views.get(interaction.runId)?.lastSequence ?? 0;
        startWatch(interaction.runId, cursor);
      } catch (error: unknown) {
        const failure = commandError(error);
        markConnectionFailure(failure);
        throw error instanceof CommandFailure
          ? error
          : new CommandFailure(failure);
      }
    },
    [markConnectionFailure, startWatch],
  );

  const activeView =
    chat.activeRunId === null ? undefined : chat.views.get(chat.activeRunId);
  const activeRun = activeView?.run;
  const canCompose =
    connection.state === "connected" &&
    !submitting &&
    (activeRun === undefined || isTerminalStatus(activeRun.status));
  const continuation =
    activeRun !== undefined && isTerminalStatus(activeRun.status);
  const promptBytes = utf8ByteLength(prompt);
  const promptOverLimit = promptBytes > MAX_PROMPT_BYTES;
  const views = useMemo(() => Array.from(chat.views.values()), [chat.views]);
  const selectedArtifacts = useMemo(
    () => selectReleasedArtifacts(activeView),
    [activeView],
  );
  const allArtifacts = useMemo(() => selectReleasedArtifacts(views), [views]);
  const activity = useMemo(() => selectOperationalActivity(views), [views]);
  const artifactItems = useMemo<ArtifactViewItem[]>(
    () =>
      selectedArtifacts.map((artifact) => {
        const previewLines = previewFor(artifact.fileName);
        return {
          id: artifact.artifactId || artifact.key,
          fileName: artifact.fileName,
          mediaType: artifact.mediaType,
          sizeLabel: artifact.sizeLabel,
          stateLabel: artifact.stateLabel,
          createdLabel: artifact.createdLabel,
          ...(previewLines === undefined ? {} : { previewLines }),
        };
      }),
    [selectedArtifacts],
  );
  const participants = useMemo<readonly AgentParticipant[]>(() => {
    if (FIXTURE_MODE && activeView !== undefined) {
      return DEMO_PARTICIPANTS;
    }
    if (activeRun === undefined) {
      return [];
    }
    return [
      {
        id: activeRun.runId,
        name: agentRoleLabel(activeRun.role),
        role: "Primary run",
        state: participantState(activeRun.status),
        icon: "lead",
      },
    ];
  }, [activeRun, activeView]);
  const attentionCount = chat.recentRuns.reduce(
    (count, run) =>
      count +
      run.pendingInteractionCount +
      (run.status === "outcome_unknown" ? 1 : 0),
    0,
  );
  const activeCount = chat.recentRuns.filter((run) =>
    ["queued", "running", "waiting", "cancelling"].includes(run.status),
  ).length;
  const title =
    activeRun === undefined ? "New work" : agentRoleLabel(activeRun.role);
  const closeWorkNavigation = useCallback(
    () => setWorkNavigationOpen(false),
    [],
  );
  const openWorkNavigation = useCallback(() => setWorkNavigationOpen(true), []);
  const selectSurface = useCallback((nextSurface: WorkspaceSurface) => {
    setWorkNavigationOpen(false);
    setSurface(nextSurface);
  }, []);

  const composer = (
    <WorkComposer
      formRef={composerFormRef}
      textareaRef={composerRef}
      prompt={prompt}
      promptBytes={promptBytes}
      promptByteLimit={MAX_PROMPT_BYTES}
      promptOverLimit={promptOverLimit}
      role={role}
      maxTurns={maxTurns}
      maxTurnsLimit={MAX_TURNS}
      mode={mode}
      targetLabel={
        activeRun === undefined ? "Colossus" : agentRoleLabel(activeRun.role)
      }
      canCompose={canCompose}
      submitting={submitting}
      continuation={continuation}
      activeWorkRunning={
        activeRun !== undefined && !isTerminalStatus(activeRun.status)
      }
      activeWorkNeedsInput={(activeView?.pendingInteractions.length ?? 0) > 0}
      error={composerError}
      onPromptChange={(nextPrompt) => {
        setPrompt(nextPrompt);
        setComposerError(null);
      }}
      onRoleChange={setRole}
      onMaxTurnsChange={(turns) => setMaxTurns(clampMaxTurns(turns))}
      onModeChange={setMode}
      onSubmit={(event) => void submitRun(event)}
    />
  );

  return (
    <div className="app-shell">
      <a className="skip-link" href="#primary-workspace">
        Skip to workspace
      </a>
      <ProductRail
        surface={surface}
        attentionCount={attentionCount}
        connectionState={connection.state}
        onSelect={selectSurface}
      />

      {surface === "work" && workNavigationOpen ? (
        <button
          className="workspace-drawer-backdrop work-navigation-backdrop"
          type="button"
          aria-label="Close work navigation"
          aria-hidden="true"
          tabIndex={-1}
          onClick={closeWorkNavigation}
        />
      ) : null}

      {surface === "work" ? (
        <WorkSidebar
          runs={chat.recentRuns}
          activeRunId={chat.activeRunId}
          query={workQuery}
          busy={listBusy}
          error={listError}
          hasMore={chat.nextPageToken !== ""}
          disabled={submitting}
          drawerOpen={workNavigationOpen}
          onQueryChange={setWorkQuery}
          onNewWork={newWork}
          onSelect={(run) => void openRun(run)}
          onLoadMore={() => void loadRuns(chat.nextPageToken, true)}
          onDrawerOpen={openWorkNavigation}
          onDrawerClose={closeWorkNavigation}
        />
      ) : (
        <ContextSidebar
          surface={surface}
          connection={connection}
          runCount={chat.recentRuns.length}
          activeCount={activeCount}
          artifactCount={allArtifacts.length}
          activityCount={activity.length}
        />
      )}

      {surface === "work" ? (
        <WorkSurface
          title={title}
          view={activeView}
          connection={connection}
          connecting={connecting}
          cancelling={cancelling}
          runLoadError={runLoadError}
          actionError={actionError}
          participants={participants}
          artifacts={artifactItems}
          composer={composer}
          workNavigationOpen={workNavigationOpen}
          onConnect={() => void connect()}
          onCancel={() => void cancelActiveRun()}
          onRespond={handleInteraction}
          onResume={() => {
            if (activeView !== undefined) {
              startWatch(activeView.run.runId, activeView.lastSequence);
            }
          }}
          onSuggestion={(suggestion) => {
            setPrompt(suggestion);
            requestAnimationFrame(() => composerRef.current?.focus());
          }}
          onOpenWorkNavigation={openWorkNavigation}
          onCloseWorkNavigation={closeWorkNavigation}
        />
      ) : (
        <OperationsSurface
          surface={surface}
          connection={connection}
          connecting={connecting}
          runs={chat.recentRuns}
          artifacts={allArtifacts}
          activity={activity}
          demoParticipants={FIXTURE_MODE ? DEMO_PARTICIPANTS : null}
          onConnect={() => void connect()}
          onOpenRun={(run) => void openRun(run)}
        />
      )}
    </div>
  );
}
