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
  addExternalTarget,
  applyManagedModelConfiguration,
  cancelRun,
  checkDesktopUpdate,
  chooseRunAttachment,
  chooseWorkspace,
  codexAuthLogin,
  codexAuthLogout,
  configureManagedRuntime,
  connectColossus,
  createRun,
  desktopReleaseChannel,
  desktopStatus,
  exportDiagnostics,
  getRun,
  importCaBundle,
  installDesktopUpdate,
  initializeDesktop,
  listWorkspaceDirectory,
  listRuns,
  readArtifactContent,
  readWorkspaceFile,
  restartManagedRuntime,
  removeCaBundle,
  removeExternalTarget,
  respondInteraction,
  runManagedSelfTest,
  selectTarget,
  setApprovalMode,
  setTerminalEnabled,
  showTerminalWindow,
  watchRun,
} from "./api";
import type { AgentParticipant, AgentWorkState } from "./components/AgentFlow";
import type {
  ArtifactPreviewLine,
  ArtifactViewItem,
} from "./components/ArtifactWorkspace";
import { ContextSidebar } from "./components/ContextSidebar";
import { OperationsSurface } from "./components/OperationsSurface";
import { OnboardingSurface } from "./components/OnboardingSurface";
import { ProductRail } from "./components/ProductRail";
import { ReleaseChannelBanner } from "./components/ReleaseChannelBanner";
import type { WorkspaceSurface } from "./components/ProductRail";
import { WorkComposer } from "./components/WorkComposer";
import { WorkSidebar } from "./components/WorkSidebar";
import { WorkSurface } from "./components/WorkSurface";
import { WorkspaceFiles } from "./components/WorkspaceFiles";
import {
  buildOperationsStudioFixture,
  buildPlanWorkflowFixture,
} from "./dev/operations-studio-fixture";
import { managedOnboardingRequired } from "./onboarding";
import {
  REMOTE_PROVIDER_TIMEOUT_MS,
  automaticProviderTimeoutMs,
} from "./providerTimeout";
import {
  agentRoleLabel,
  safeDisplayLabel,
  selectOperationalActivity,
  selectReleasedArtifacts,
} from "./presenters";
import {
  MAX_CONVERSATION_RUNS,
  MAX_PROMPT_BYTES,
  MAX_TURNS,
  chatReducer,
  clampMaxTurns,
  connectionStateForError,
  initialChatState,
  isConnectionError,
  isPromptWithinByteLimit,
  operationFingerprint,
  selectConversationViews,
  stableIdempotentAttempt,
  utf8ByteLength,
  withBoundedEntry,
} from "./state";
import type { IdempotentAttempt } from "./state";
import {
  TargetRouteRegistry,
  selectedTargetRouteChanged,
  watchDurableRun,
} from "./target-routing";
import type { TargetRoute } from "./target-routing";
import type {
  ApplyManagedModelConfigurationRequest,
  ApprovalMode,
  ArtifactReference,
  CommandError,
  ConfigureManagedRuntimeRequest,
  ConnectionStatus,
  CreateRunRequest,
  DesktopStatus,
  Interaction,
  InteractionAnswer,
  Run,
  RunMode,
  RunStatus,
  TerminalKind,
} from "./types";
import { USE_CONFIGURED_MAX_TURNS, isTerminalStatus } from "./types";
import {
  listFixtureWorkspaceDirectory,
  readFixtureWorkspaceFile,
} from "./dev/workspace-files-fixture";

const FIXTURE_SCENARIO = new URLSearchParams(window.location.search).get(
  "fixture",
);
const FIXTURE_MODE =
  import.meta.env.DEV &&
  (FIXTURE_SCENARIO === "operations-studio" ||
    FIXTURE_SCENARIO === "interaction-question" ||
    FIXTURE_SCENARIO === "plan-workflow");

const INITIAL_CONNECTION: ConnectionStatus = FIXTURE_MODE
  ? {
      state: "connected",
      message:
        "Development showcase connected to a deterministic local fixture.",
      targetId: "fixture-managed-local",
    }
  : {
      state: "disconnected",
      message: "Connecting to the local Colossus agent…",
      targetId: null,
    };

const INITIAL_DESKTOP: DesktopStatus = {
  releaseChannel: "development",
  connection: INITIAL_CONNECTION,
  targets: FIXTURE_MODE
    ? [
        {
          targetId: "fixture-managed-local",
          kind: "managed_local",
          label: "Managed Local",
          state: "ready",
          message: "Fixture runtime ready.",
          selected: true,
          terminalAvailable: true,
          workspace: {
            workspaceId: "fixture-workspace",
            displayName: "Colossus",
            displayPath: "~/tools/Colossus",
          },
          failureCode: null,
        },
      ]
    : [],
  selectedTargetId: FIXTURE_MODE ? "fixture-managed-local" : null,
  managedState: FIXTURE_MODE ? "ready" : "starting",
  workspace: FIXTURE_MODE
    ? {
        workspaceId: "fixture-workspace",
        displayName: "Colossus",
        displayPath: "~/tools/Colossus",
      }
    : null,
  provider: FIXTURE_MODE
    ? {
        configured: true,
        kind: "openai_compatible",
        model: "fixture",
      }
    : { configured: false, kind: null, model: "" },
  codexAuth: {
    state: FIXTURE_MODE ? "signed_in" : "signed_out",
    message: FIXTURE_MODE
      ? "Fixture ChatGPT account connected."
      : "Sign in with ChatGPT to use the Codex subscription provider.",
  },
  managedModelConfiguration: {
    providers: FIXTURE_MODE
      ? [
          {
            profile: "primary-provider",
            providerKind: "openai_compatible",
            baseUrl: "https://openrouter.ai/api/v1",
            hasCredential: false,
            timeoutMs: null,
            effectiveTimeoutMs: REMOTE_PROVIDER_TIMEOUT_MS,
          },
        ]
      : [],
    models: FIXTURE_MODE
      ? [
          {
            profile: "primary",
            providerProfile: "primary-provider",
            model: "fixture",
            contextWindowTokens: 128_000,
            maxOutputTokens: 16_000,
            reasoningEffort: null,
            capabilities: { toolCalls: true, streaming: true },
          },
        ]
      : [],
    roles: FIXTURE_MODE ? { primary: "primary" } : {},
  },
  accessProfile: "development",
  approvalMode: "ask",
  terminalEnabled: false,
  additionalCaBundle: {
    configured: false,
    certificateCount: 0,
    fingerprintsSha256: [],
  },
  capabilities: {
    delegation: false,
    skills: false,
    tui: FIXTURE_MODE,
    shellTerminal: FIXTURE_MODE,
    files: FIXTURE_MODE,
    artifacts: FIXTURE_MODE,
    planContinuation: FIXTURE_MODE,
    updateAvailable: false,
    agentWorkflows: false,
    attachments: false,
  },
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

interface RoutedAttempt {
  targetId: string;
  attempt: IdempotentAttempt;
}

interface PlanRevisionTarget {
  sourceRunId: string;
  planId: string;
  revision: number;
}

export default function App() {
  const [chat, dispatch] = useReducer(
    chatReducer,
    FIXTURE_MODE
      ? FIXTURE_SCENARIO === "plan-workflow"
        ? buildPlanWorkflowFixture()
        : buildOperationsStudioFixture(
            FIXTURE_SCENARIO === "interaction-question"
              ? "user_prompt"
              : "approval",
          )
      : initialChatState,
  );
  const chatRef = useRef(chat);
  const [desktop, setDesktop] = useState<DesktopStatus>(INITIAL_DESKTOP);
  const [releaseChannel, setReleaseChannel] = useState(
    INITIAL_DESKTOP.releaseChannel,
  );
  const desktopRef = useRef(desktop);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [surface, setSurface] = useState<WorkspaceSurface>("work");
  const [workNavigationOpen, setWorkNavigationOpen] = useState(false);
  const [workQuery, setWorkQuery] = useState("");
  const connection = desktop.connection;
  const [connecting, setConnecting] = useState(!FIXTURE_MODE);
  const [listBusy, setListBusy] = useState(false);
  const [listError, setListError] = useState("");
  const [runLoadError, setRunLoadError] = useState("");
  const [prompt, setPrompt] = useState("");
  const [attachments, setAttachments] = useState<ArtifactReference[]>([]);
  const [attachmentBusy, setAttachmentBusy] = useState(false);
  const [artifactPreviews, setArtifactPreviews] = useState<
    ReadonlyMap<string, readonly ArtifactPreviewLine[]>
  >(new Map());
  const [artifactPreviewFailures, setArtifactPreviewFailures] = useState<
    ReadonlyMap<string, string>
  >(new Map());
  const [artifactPreviewsLoading, setArtifactPreviewsLoading] = useState<
    ReadonlySet<string>
  >(new Set());
  const [role, setRole] = useState("primary");
  const [mode, setMode] = useState<RunMode>("execute");
  const [planRevision, setPlanRevision] = useState<PlanRevisionTarget | null>(
    null,
  );
  const [maxTurns, setMaxTurns] = useState(USE_CONFIGURED_MAX_TURNS);
  const [submitting, setSubmitting] = useState(false);
  const [approvalModeChanging, setApprovalModeChanging] = useState(false);
  const [composerError, setComposerError] = useState<CommandError | null>(null);
  const [actionError, setActionError] = useState<CommandError | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");
  const [cancelling, setCancelling] = useState(false);
  const watchedRuns = useRef(new Map<string, symbol>());
  const targetRoutes = useRef<TargetRouteRegistry | null>(null);
  if (targetRoutes.current === null) {
    targetRoutes.current = new TargetRouteRegistry();
    if (FIXTURE_MODE) {
      const route = targetRoutes.current.activate(
        "fixture-managed-local",
        "managed_local",
      );
      targetRoutes.current.bindRuns(chat.views.keys(), route);
    }
  }
  const connectingRef = useRef(false);
  const submitInFlight = useRef(false);
  const createAttempt = useRef<RoutedAttempt | null>(null);
  const cancelAttempts = useRef(new Map<string, IdempotentAttempt>());
  const responseAttempts = useRef(new Map<string, IdempotentAttempt>());
  const listRequest = useRef<symbol | null>(null);
  const cancelRequest = useRef<symbol | null>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const composerFormRef = useRef<HTMLFormElement>(null);

  useEffect(() => {
    chatRef.current = chat;
  }, [chat]);

  useEffect(() => {
    desktopRef.current = desktop;
  }, [desktop]);

  const markConnectionFailure = useCallback(
    (failure: CommandError, route?: TargetRoute) => {
      if (
        isConnectionError(failure) &&
        (route === undefined || targetRoutes.current?.isCurrent(route) === true)
      ) {
        setDesktop((current) => ({
          ...current,
          connection: {
            state: connectionStateForError(failure),
            message: failure.message,
            targetId: current.selectedTargetId,
          },
        }));
      }
    },
    [],
  );

  const invalidateTargetRoute = useCallback(() => {
    targetRoutes.current?.invalidate();
    watchedRuns.current.clear();
  }, []);

  const loadRuns = useCallback(
    async (pageToken: string, append: boolean, explicitRoute?: TargetRoute) => {
      if (FIXTURE_MODE) {
        return true;
      }
      const route = explicitRoute ?? targetRoutes.current?.capture() ?? null;
      if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
        setListError("Select a connected Colossus target first.");
        return false;
      }
      const requestToken = Symbol("list-runs");
      listRequest.current = requestToken;
      setListBusy(true);
      setListError("");
      try {
        const page = await listRuns(route.targetId, { pageToken });
        if (
          targetRoutes.current?.isCurrent(route) !== true ||
          listRequest.current !== requestToken
        ) {
          return false;
        }
        targetRoutes.current.bindRuns(
          page.runs.map((run) => run.runId),
          route,
        );
        dispatch({
          type: append ? "append_recent" : "replace_recent",
          runs: page.runs,
          nextPageToken: page.nextPageToken,
        });
        return true;
      } catch (error: unknown) {
        if (
          targetRoutes.current?.isCurrent(route) !== true ||
          listRequest.current !== requestToken
        ) {
          return false;
        }
        const failure = commandError(error);
        markConnectionFailure(failure, route);
        setListError(failure.message);
        return false;
      } finally {
        if (listRequest.current === requestToken) {
          listRequest.current = null;
          setListBusy(false);
        }
      }
    },
    [markConnectionFailure],
  );

  const startWatch = useCallback(
    (runId: string, afterSequence: number, explicitRoute?: TargetRoute) => {
      const route =
        explicitRoute ?? targetRoutes.current?.routeForRun(runId) ?? null;
      if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const watchKey = `${route.targetId}:${route.generation}:${runId}`;
      if (FIXTURE_MODE || watchedRuns.current.has(watchKey)) {
        return;
      }
      const token = Symbol(runId);
      watchedRuns.current.set(watchKey, token);
      dispatch({ type: "watch_started", runId });

      void watchDurableRun({
        route,
        runId,
        afterSequence,
        isCurrent: (candidate) =>
          targetRoutes.current?.isCurrent(candidate) === true,
        watch: (targetId, watchedRunId, cursor, onEvent) =>
          watchRun(
            targetId,
            { runId: watchedRunId, afterSequence: cursor },
            onEvent,
          ),
        getRun: (targetId, watchedRunId) =>
          getRun(targetId, { runId: watchedRunId }),
        normalizeError: commandError,
        canRecover: (failure, candidate) =>
          candidate.kind === "managed_local" &&
          failure.retryable &&
          isConnectionError(failure),
        onUpdate: (update) => {
          dispatch({ type: "ingest_update", update });
        },
        onHydrate: (details) => {
          targetRoutes.current?.bindRun(details.run.runId, route);
          dispatch({ type: "hydrate_run", details });
        },
      })
        .then((result) => {
          if (
            result.type === "stale" ||
            targetRoutes.current?.isCurrent(route) !== true
          ) {
            return;
          }
          if (result.type === "complete") {
            dispatch({ type: "watch_complete", runId });
            return;
          }
          markConnectionFailure(result.error, route);
          dispatch({ type: "watch_error", runId, error: result.error });
        })
        .finally(() => {
          if (watchedRuns.current.get(watchKey) === token) {
            watchedRuns.current.delete(watchKey);
          }
        });
    },
    [markConnectionFailure],
  );

  const acceptDesktopStatus = useCallback(
    async (status: DesktopStatus, resetWork: boolean) => {
      const previousStatus = desktopRef.current;
      desktopRef.current = status;
      setDesktop(status);
      setReleaseChannel(status.releaseChannel);
      const requiresOnboarding = managedOnboardingRequired(status);
      setShowOnboarding((current) => current || requiresOnboarding);
      if (
        status.connection.state !== "connected" ||
        status.selectedTargetId === null
      ) {
        invalidateTargetRoute();
        if (resetWork) {
          dispatch({ type: "reset" });
        }
        return;
      }
      const selectedTarget = status.targets.find(
        (target) => target.targetId === status.selectedTargetId,
      );
      if (
        selectedTarget === undefined ||
        selectedTarget.state !== "ready" ||
        status.connection.targetId !== status.selectedTargetId
      ) {
        invalidateTargetRoute();
        setListError("The selected Colossus target is unavailable.");
        if (resetWork) {
          dispatch({ type: "reset" });
        }
        return;
      }
      const currentRoute = targetRoutes.current?.capture() ?? null;
      if (
        !resetWork &&
        currentRoute !== null &&
        currentRoute.targetId === selectedTarget.targetId &&
        currentRoute.kind === selectedTarget.kind &&
        !selectedTargetRouteChanged(previousStatus, status)
      ) {
        return;
      }
      const route = targetRoutes.current?.activate(
        selectedTarget.targetId,
        selectedTarget.kind,
      );
      watchedRuns.current.clear();
      if (route === undefined) {
        return;
      }
      if (resetWork) {
        dispatch({ type: "reset" });
      } else {
        targetRoutes.current?.bindRuns(chatRef.current.views.keys(), route);
      }
      const runsLoaded = await loadRuns("", false, route);
      if (
        runsLoaded &&
        !resetWork &&
        targetRoutes.current?.isCurrent(route) === true
      ) {
        const activeRunId = chatRef.current.activeRunId;
        const activeView =
          activeRunId === null
            ? undefined
            : chatRef.current.views.get(activeRunId);
        if (
          activeView !== undefined &&
          !isTerminalStatus(activeView.run.status)
        ) {
          startWatch(activeView.run.runId, activeView.lastSequence, route);
        }
      }
    },
    [invalidateTargetRoute, loadRuns, startWatch],
  );

  const connect = useCallback(
    async (targetId?: string) => {
      if (managedOnboardingRequired(desktopRef.current)) {
        setActionError(null);
        setShowOnboarding(true);
        return;
      }
      if (FIXTURE_MODE) {
        setDesktop(INITIAL_DESKTOP);
        setConnecting(false);
        return;
      }
      if (connectingRef.current || submitInFlight.current) {
        return;
      }
      connectingRef.current = true;
      invalidateTargetRoute();
      setConnecting(true);
      setActionError(null);
      try {
        await connectColossus(targetId);
        const status = await desktopStatus();
        await acceptDesktopStatus(status, false);
      } catch (error: unknown) {
        const failure = commandError(error);
        markConnectionFailure(failure);
        setActionError(failure);
        try {
          await acceptDesktopStatus(await desktopStatus(), false);
        } catch (statusError: unknown) {
          markConnectionFailure(commandError(statusError));
        }
      } finally {
        connectingRef.current = false;
        setConnecting(false);
      }
    },
    [acceptDesktopStatus, invalidateTargetRoute, markConnectionFailure],
  );

  useEffect(() => {
    if (FIXTURE_MODE) {
      setConnecting(false);
      return;
    }
    let cancelled = false;
    connectingRef.current = true;
    setConnecting(true);
    void desktopReleaseChannel()
      .then((channel) => {
        if (!cancelled) {
          setReleaseChannel(channel);
        }
      })
      .catch(() => {
        // initialize_desktop returns the same native channel when setup succeeds.
      });
    void initializeDesktop()
      .then(async (status) => {
        if (!cancelled) {
          await acceptDesktopStatus(status, true);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          const failure = commandError(error);
          markConnectionFailure(failure);
          setActionError(failure);
        }
      })
      .finally(() => {
        if (!cancelled) {
          connectingRef.current = false;
          setConnecting(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [acceptDesktopStatus, markConnectionFailure]);

  useEffect(() => {
    if (FIXTURE_MODE) {
      return;
    }
    let cancelled = false;
    let polling = false;
    const refresh = async () => {
      if (
        cancelled ||
        polling ||
        connectingRef.current ||
        submitInFlight.current
      ) {
        return;
      }
      polling = true;
      try {
        const status = await desktopStatus();
        if (!cancelled && !connectingRef.current && !submitInFlight.current) {
          await acceptDesktopStatus(status, false);
        }
      } catch {
        // Interactive commands and watches surface selected-target failures.
        // Periodic health refresh remains quiet and tries again on the next tick.
      } finally {
        polling = false;
      }
    };
    const timer = window.setInterval(() => void refresh(), 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [acceptDesktopStatus]);

  async function openRun(run: Run) {
    if (submitInFlight.current || connectingRef.current) {
      return;
    }
    if (FIXTURE_MODE) {
      setPlanRevision(null);
      setWorkNavigationOpen(false);
      setSurface("work");
      dispatch({ type: "upsert_run", run });
      dispatch({ type: "select_run", runId: run.runId });
      setRunLoadError("");
      setActionError(null);
      return;
    }
    const route = targetRoutes.current?.routeForRun(run.runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      setRunLoadError("Select a connected Colossus target first.");
      return;
    }
    targetRoutes.current.bindRun(run.runId, route);
    setPlanRevision(null);
    setWorkNavigationOpen(false);
    setSurface("work");
    dispatch({ type: "upsert_run", run });
    dispatch({ type: "select_run", runId: run.runId });
    setRunLoadError("");
    setActionError(null);
    const existingCursor =
      chatRef.current.views.get(run.runId)?.lastSequence ?? 0;
    try {
      const details = await getRun(route.targetId, { runId: run.runId });
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      targetRoutes.current.bindRun(details.run.runId, route);
      dispatch({ type: "hydrate_run", details });
      startWatch(run.runId, existingCursor, route);

      try {
        const history = await listRuns(route.targetId, {
          sessionId: details.run.sessionId,
          pageToken: "",
        });
        if (targetRoutes.current?.isCurrent(route) !== true) {
          return;
        }
        const sessionRuns = history.runs
          .filter((candidate) => candidate.sessionId === details.run.sessionId)
          .slice(0, MAX_CONVERSATION_RUNS);
        targetRoutes.current.bindRuns(
          sessionRuns.map((candidate) => candidate.runId),
          route,
        );
        for (const historicalRun of [...sessionRuns].reverse()) {
          dispatch({ type: "upsert_run", run: historicalRun });
        }
        for (const historicalRun of sessionRuns) {
          if (historicalRun.runId === details.run.runId) {
            continue;
          }
          const cursor =
            chatRef.current.views.get(historicalRun.runId)?.lastSequence ?? 0;
          startWatch(historicalRun.runId, cursor, route);
        }
      } catch (historyError: unknown) {
        if (targetRoutes.current?.isCurrent(route) === true) {
          const failure = commandError(historyError);
          markConnectionFailure(failure, route);
          setRunLoadError(
            "This work opened, but its earlier turns could not be loaded.",
          );
        }
      }
    } catch (error: unknown) {
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const failure = commandError(error);
      markConnectionFailure(failure, route);
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
    setPlanRevision(null);
    setAttachments([]);
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  async function submitRun(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitInFlight.current || connectingRef.current) {
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
    const continuationView =
      planRevision === null
        ? currentView
        : chat.views.get(planRevision.sourceRunId);
    const route =
      continuationView === undefined
        ? (targetRoutes.current?.capture() ?? null)
        : (targetRoutes.current?.routeForRun(continuationView.run.runId) ??
          null);
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      setComposerError({
        ...FALLBACK_ACTION_ERROR,
        code: "disconnected",
        message: "Select a connected Colossus target first.",
      });
      return;
    }
    const sessionId =
      continuationView !== undefined &&
      isTerminalStatus(continuationView.run.status)
        ? continuationView.run.sessionId
        : undefined;
    const effectiveMode: RunMode = planRevision === null ? mode : "plan";
    const fingerprint = operationFingerprint([
      cleanPrompt,
      route.targetId,
      sessionId ?? "",
      cleanRole,
      effectiveMode,
      maxTurns,
      planRevision?.sourceRunId ?? "",
      planRevision?.planId ?? "",
      planRevision?.revision ?? 0,
      ...attachments.map((attachment) => attachment.artifactId),
    ]);
    const previousRoutedAttempt = createAttempt.current;
    const previousAttempt =
      previousRoutedAttempt !== null &&
      previousRoutedAttempt.targetId === route.targetId
        ? previousRoutedAttempt.attempt
        : null;
    const attempt = stableIdempotentAttempt(previousAttempt, fingerprint);
    createAttempt.current = {
      targetId: route.targetId,
      attempt,
    };
    const commonRequest: CreateRunRequest = {
      prompt: cleanPrompt,
      artifactIds: attachments.map((attachment) => attachment.artifactId),
      role: cleanRole,
      mode: effectiveMode,
      maxTurns,
      idempotencyKey: attempt.key,
      ...(planRevision === null
        ? {}
        : {
            planAction: {
              type: "revise" as const,
              sourceRunId: planRevision.sourceRunId,
              expectedRevision: planRevision.revision,
            },
          }),
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
          title: safeDisplayLabel(cleanPrompt, "Untitled work", 80),
          role: cleanRole,
          mode: effectiveMode,
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
                planRevision === null
                  ? "Showcase response: the request was accepted by the local Operations Studio fixture. Live builds send this through the scoped native command boundary."
                  : "The selected Plan was revised in this chat and saved as a new durable draft revision.",
              ...(planRevision === null
                ? {}
                : {
                    planId: planRevision.planId,
                    planRevision: planRevision.revision + 1,
                    planStatus: "draft" as const,
                  }),
              profile: "desktop-showcase",
              modelProfile: "desktop-showcase",
              providerProfile: "fixture-provider",
              model: "fixture",
              elapsedSeconds: 0.2,
            },
          },
          etag: `fixture-etag-${runId}`,
          selectedSkills: [],
        };
        createAttempt.current = null;
        setPrompt("");
        setPlanRevision(null);
        setAttachments([]);
        targetRoutes.current?.bindRun(runId, route);
        dispatch({ type: "upsert_run", run });
        dispatch({ type: "record_local_prompt", runId, prompt: cleanPrompt });
        dispatch({ type: "select_run", runId });
        return;
      }

      if (route === null) {
        return;
      }
      const run = await createRun(route.targetId, request);
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      createAttempt.current = null;
      setPrompt("");
      setPlanRevision(null);
      setAttachments([]);
      targetRoutes.current.bindRun(run.runId, route);
      dispatch({ type: "upsert_run", run });
      dispatch({
        type: "record_local_prompt",
        runId: run.runId,
        prompt: cleanPrompt,
      });
      dispatch({ type: "select_run", runId: run.runId });
      startWatch(run.runId, 0, route);
    } catch (error: unknown) {
      if (route !== null && targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const failure = commandError(error);
      markConnectionFailure(failure, route ?? undefined);
      setComposerError(failure);
    } finally {
      submitInFlight.current = false;
      setSubmitting(false);
    }
  }

  function beginPlanRevision(
    sourceRunId: string,
    planId: string,
    revision: number,
  ) {
    if (submitInFlight.current || connectingRef.current) {
      return;
    }
    const source = chatRef.current.views.get(sourceRunId);
    if (source === undefined || !isTerminalStatus(source.run.status)) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "plan_source_unavailable",
        message: "Reload this work before revising its Plan.",
      });
      return;
    }
    setPlanRevision({ sourceRunId, planId, revision });
    setMode("plan");
    setPrompt("");
    setComposerError(null);
    setActionError(null);
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  async function executePlan(
    sourceRunId: string,
    planId: string,
    revision: number,
    strategy: { type: "direct" } | { type: "goal"; maxIterations: number },
  ) {
    if (submitInFlight.current || connectingRef.current) {
      return;
    }
    const source = chatRef.current.views.get(sourceRunId);
    const route = targetRoutes.current?.routeForRun(sourceRunId) ?? null;
    if (
      source === undefined ||
      !isTerminalStatus(source.run.status) ||
      route === null ||
      targetRoutes.current?.isCurrent(route) !== true
    ) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "plan_source_unavailable",
        message: "Reload this work before executing its Plan.",
      });
      return;
    }
    const actionPrompt =
      strategy.type === "direct"
        ? `Approve and execute Plan revision ${revision} once.`
        : `Approve and run Plan revision ${revision} as a bounded Goal with ${strategy.maxIterations} iterations.`;
    const fingerprint = operationFingerprint([
      actionPrompt,
      route.targetId,
      sourceRunId,
      planId,
      revision,
      strategy.type,
      strategy.type === "goal" ? strategy.maxIterations : 0,
      source.run.role,
      maxTurns,
    ]);
    const previousRoutedAttempt = createAttempt.current;
    const previousAttempt =
      previousRoutedAttempt !== null &&
      previousRoutedAttempt.targetId === route.targetId
        ? previousRoutedAttempt.attempt
        : null;
    const attempt = stableIdempotentAttempt(previousAttempt, fingerprint);
    createAttempt.current = {
      targetId: route.targetId,
      attempt,
    };
    const request: CreateRunRequest = {
      prompt: actionPrompt,
      sessionId: source.run.sessionId,
      role: source.run.role,
      mode: "execute",
      planAction: {
        type: "execute",
        sourceRunId,
        expectedRevision: revision,
        strategy,
      },
      maxTurns,
      idempotencyKey: attempt.key,
    };

    submitInFlight.current = true;
    setSubmitting(true);
    setActionError(null);
    setComposerError(null);
    try {
      let run: Run;
      if (FIXTURE_MODE) {
        const now = new Date().toISOString();
        const runId = `fixture-plan-execution-${Date.now()}`;
        run = {
          runId,
          sessionId: source.run.sessionId,
          title: actionPrompt,
          role: source.run.role,
          mode: "execute",
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
                strategy.type === "direct"
                  ? "The selected Plan completed as one policy-bound run."
                  : "The selected Plan was consumed into bounded Goal Mode.",
              planId,
              planRevision: revision + 2,
              planStatus: "executed",
              ...(strategy.type === "goal"
                ? { goalId: `goal-fixture-${Date.now()}` }
                : {}),
              profile: "desktop-showcase",
              modelProfile: "desktop-showcase",
              providerProfile: "fixture-provider",
              model: "fixture",
              elapsedSeconds: 0.2,
            },
          },
          etag: `fixture-etag-${runId}`,
          selectedSkills: [],
        };
      } else {
        run = await createRun(route.targetId, request);
      }
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      createAttempt.current = null;
      setPlanRevision(null);
      targetRoutes.current.bindRun(run.runId, route);
      dispatch({ type: "upsert_run", run });
      dispatch({
        type: "record_local_prompt",
        runId: run.runId,
        prompt: actionPrompt,
      });
      dispatch({ type: "select_run", runId: run.runId });
      if (!FIXTURE_MODE) {
        startWatch(run.runId, 0, route);
      }
    } catch (error: unknown) {
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const failure = commandError(error);
      markConnectionFailure(failure, route);
      setActionError(failure);
    } finally {
      submitInFlight.current = false;
      setSubmitting(false);
    }
  }

  async function chooseAttachment() {
    const route = targetRoutes.current?.capture() ?? null;
    if (
      route === null ||
      targetRoutes.current?.isCurrent(route) !== true ||
      attachmentBusy
    ) {
      return;
    }
    setAttachmentBusy(true);
    setComposerError(null);
    try {
      const attachment = await chooseRunAttachment(route.targetId);
      if (
        attachment !== null &&
        targetRoutes.current?.isCurrent(route) === true
      ) {
        setAttachments((current) =>
          current.some((item) => item.artifactId === attachment.artifactId)
            ? current
            : [...current, attachment].slice(0, 16),
        );
      }
    } catch (error: unknown) {
      setComposerError(commandError(error));
    } finally {
      setAttachmentBusy(false);
    }
  }

  async function loadArtifactPreview(artifactId: string) {
    if (
      FIXTURE_MODE ||
      artifactId.length === 0 ||
      artifactPreviews.has(artifactId) ||
      artifactPreviewsLoading.has(artifactId)
    ) {
      return;
    }
    const route = targetRoutes.current?.capture() ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      return;
    }
    setArtifactPreviewsLoading((current) => {
      const next = new Set(current);
      next.add(artifactId);
      return next;
    });
    setArtifactPreviewFailures((current) => {
      const next = new Map(current);
      next.delete(artifactId);
      return next;
    });
    try {
      const content = await readArtifactContent(route.targetId, artifactId);
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const lines = content.text.split(/\r?\n/).map((text, index) => ({
        number: index + 1,
        kind: "context" as const,
        text,
      }));
      setArtifactPreviews((current) => {
        const next = new Map(current);
        next.set(artifactId, lines);
        return next;
      });
    } catch (error: unknown) {
      if (targetRoutes.current?.isCurrent(route) === true) {
        const failure = commandError(error);
        setArtifactPreviewFailures((current) => {
          const next = new Map(current);
          next.set(artifactId, failure.message);
          return next;
        });
      }
    } finally {
      setArtifactPreviewsLoading((current) => {
        const next = new Set(current);
        next.delete(artifactId);
        return next;
      });
    }
  }

  async function cancelActiveRun() {
    if (connectingRef.current) {
      return;
    }
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

    const route = targetRoutes.current?.routeForRun(runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "disconnected",
        message: "The active run is no longer bound to this target.",
      });
      return;
    }
    const attemptKey = `${route.targetId}:${runId}`;
    const fingerprint = operationFingerprint([route.targetId, runId, "cancel"]);
    const attempt = stableIdempotentAttempt(
      cancelAttempts.current.get(attemptKey) ?? null,
      fingerprint,
    );
    cancelAttempts.current = withBoundedEntry(
      cancelAttempts.current,
      attemptKey,
      attempt,
    );
    const requestToken = Symbol("cancel-run");
    cancelRequest.current = requestToken;
    setCancelling(true);
    setActionError(null);
    try {
      const run = await cancelRun(route.targetId, {
        runId,
        idempotencyKey: attempt.key,
      });
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      dispatch({ type: "upsert_run", run });
      startWatch(runId, activeView.lastSequence, route);
    } catch (error: unknown) {
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const failure = commandError(error);
      markConnectionFailure(failure, route);
      setActionError(failure);
    } finally {
      if (cancelRequest.current === requestToken) {
        cancelRequest.current = null;
        setCancelling(false);
      }
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
      if (connectingRef.current) {
        throw new CommandFailure({
          ...FALLBACK_ACTION_ERROR,
          code: "busy",
          message: "Wait for the current connection operation to finish.",
        });
      }
      const route =
        targetRoutes.current?.routeForRun(interaction.runId) ?? null;
      if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
        throw new CommandFailure({
          ...FALLBACK_ACTION_ERROR,
          code: "disconnected",
          message: "The interaction is no longer bound to this target.",
        });
      }
      const fingerprint = operationFingerprint([
        route.targetId,
        interaction.runId,
        interaction.interactionId,
        interaction.etag,
        response,
      ]);
      const attemptKey = `${route.targetId}:${interaction.interactionId}`;
      const attempt = stableIdempotentAttempt(
        responseAttempts.current.get(attemptKey) ?? null,
        fingerprint,
      );
      responseAttempts.current = withBoundedEntry(
        responseAttempts.current,
        attemptKey,
        attempt,
      );

      try {
        const resolved = await respondInteraction(route.targetId, {
          runId: interaction.runId,
          interactionId: interaction.interactionId,
          etag: interaction.etag,
          idempotencyKey: attempt.key,
          response,
        });
        if (targetRoutes.current?.isCurrent(route) !== true) {
          return;
        }
        responseAttempts.current.delete(attemptKey);
        dispatch({ type: "interaction_resolved", interaction: resolved });
        const cursor =
          chatRef.current.views.get(interaction.runId)?.lastSequence ?? 0;
        startWatch(interaction.runId, cursor, route);
      } catch (error: unknown) {
        if (targetRoutes.current?.isCurrent(route) !== true) {
          return;
        }
        const failure = commandError(error);
        markConnectionFailure(failure, route);
        throw error instanceof CommandFailure
          ? error
          : new CommandFailure(failure);
      }
    },
    [markConnectionFailure, startWatch],
  );

  async function handleChooseWorkspace() {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    const generation = targetRoutes.current?.captureGeneration() ?? 0;
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        setShowOnboarding(true);
        return;
      }
      const workspace = await chooseWorkspace();
      if (
        workspace === null ||
        targetRoutes.current?.isGenerationCurrent(generation) !== true
      ) {
        return;
      }
      invalidateTargetRoute();
      const status = await desktopStatus();
      const next =
        status.workspace === null
          ? {
              ...status,
              workspace,
              managedState: "needs_provider" as const,
            }
          : status;
      await acceptDesktopStatus(next, true);
      setShowOnboarding(true);
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function resyncDesktopAfterFailedMutation() {
    if (FIXTURE_MODE) {
      return;
    }
    try {
      await acceptDesktopStatus(await desktopStatus(), false);
    } catch (error: unknown) {
      markConnectionFailure(commandError(error));
    }
  }

  async function handleConfigureManaged(
    request: ConfigureManagedRuntimeRequest,
  ): Promise<boolean> {
    if (connectingRef.current || submitInFlight.current) {
      return false;
    }
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        setDesktop((current) => ({
          ...current,
          terminalEnabled: false,
          provider: {
            configured: true,
            kind: request.providerKind,
            model: request.model,
          },
          accessProfile: request.accessProfile,
        }));
        setShowOnboarding(false);
        return true;
      }
      const status = await configureManagedRuntime(request);
      await acceptDesktopStatus(status, true);
      setShowOnboarding(false);
      return status.connection.state === "connected";
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
      return false;
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleApplyManagedModelConfiguration(
    request: ApplyManagedModelConfigurationRequest,
  ): Promise<boolean> {
    if (connectingRef.current || submitInFlight.current) {
      return false;
    }
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        const primaryProfile = request.roles.primary;
        const primaryModel = request.models.find(
          (model) => model.profile === primaryProfile,
        );
        const primaryProvider = request.providers.find(
          (provider) => provider.profile === primaryModel?.providerProfile,
        );
        setDesktop((current) => ({
          ...current,
          provider: {
            configured:
              primaryModel !== undefined && primaryProvider !== undefined,
            kind: primaryProvider?.providerKind ?? null,
            model: primaryModel?.model ?? "",
          },
          managedModelConfiguration: {
            providers: request.providers.map((provider) => ({
              profile: provider.profile,
              providerKind: provider.providerKind,
              baseUrl: provider.baseUrl,
              hasCredential: provider.credentialAction !== "none",
              timeoutMs: provider.timeoutMs,
              effectiveTimeoutMs:
                provider.timeoutMs ??
                automaticProviderTimeoutMs(provider.baseUrl),
            })),
            models: request.models,
            roles: request.roles,
          },
          accessProfile: request.accessProfile,
        }));
        setShowOnboarding(false);
        return true;
      }
      const status = await applyManagedModelConfiguration(request);
      await acceptDesktopStatus(status, true);
      setShowOnboarding(false);
      return status.connection.state === "connected";
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
      return false;
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleCodexLogin() {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      const codexAuth = FIXTURE_MODE
        ? {
            state: "signed_in" as const,
            message: "Fixture ChatGPT account connected.",
          }
        : await codexAuthLogin();
      const status = { ...desktopRef.current, codexAuth };
      desktopRef.current = status;
      setDesktop(status);
    } catch (error: unknown) {
      setActionError(commandError(error));
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleCodexLogout() {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      const codexAuth = FIXTURE_MODE
        ? {
            state: "signed_out" as const,
            message:
              "Sign in with ChatGPT to use the Codex subscription provider.",
          }
        : await codexAuthLogout();
      const status = { ...desktopRef.current, codexAuth };
      desktopRef.current = status;
      setDesktop(status);
    } catch (error: unknown) {
      setActionError(commandError(error));
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleManagedSelfTest() {
    if (FIXTURE_MODE) {
      return;
    }
    if (connectingRef.current || submitInFlight.current) {
      throw new Error("Another desktop operation is already in progress.");
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      await runManagedSelfTest();
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
      throw new Error(failure.message);
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleCheckDesktopUpdate() {
    if (FIXTURE_MODE) {
      setUpdateMessage(
        "Development fixtures do not advertise an update channel.",
      );
      return;
    }
    setUpdateChecking(true);
    setUpdateMessage("");
    setActionError(null);
    try {
      const result = await checkDesktopUpdate();
      setDesktop((current) => ({
        ...current,
        capabilities: {
          ...current.capabilities,
          updateAvailable: result.available,
        },
      }));
      setUpdateMessage(
        !result.configured
          ? "This build does not advertise an update channel."
          : result.available && result.version !== null
            ? `Colossus Desktop ${result.version} is available.`
            : `Colossus Desktop ${result.currentVersion} is up to date.`,
      );
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
      setUpdateMessage("The signed update channel could not be checked.");
    } finally {
      setUpdateChecking(false);
    }
  }

  async function handleInstallDesktopUpdate() {
    if (FIXTURE_MODE) {
      return;
    }
    setUpdateChecking(true);
    setActionError(null);
    try {
      const installed = await installDesktopUpdate();
      setUpdateMessage(
        installed
          ? "The verified update was installed. Restart Colossus Desktop to use it."
          : "The update was not installed.",
      );
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
      setUpdateMessage("The update could not be verified or installed.");
    } finally {
      setUpdateChecking(false);
    }
  }

  async function handleSelectTarget(targetId: string) {
    if (
      targetId === desktop.selectedTargetId ||
      connectingRef.current ||
      submitInFlight.current
    ) {
      return;
    }
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      const status = FIXTURE_MODE ? desktop : await selectTarget(targetId);
      await acceptDesktopStatus(status, true);
      setSurface("work");
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleAddExternalTarget(): Promise<boolean> {
    if (connectingRef.current || submitInFlight.current) {
      return false;
    }
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        return false;
      }
      const imported = await addExternalTarget();
      if (imported === null || imported.selectedTargetId === null) {
        await resyncDesktopAfterFailedMutation();
        return false;
      }
      await acceptDesktopStatus(imported, true);
      if (imported.connection.state === "connected") {
        setShowOnboarding(false);
        return true;
      }
      return false;
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
      return false;
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleRemoveExternalTarget(targetId: string) {
    const target = desktopRef.current.targets.find(
      (candidate) => candidate.targetId === targetId,
    );
    if (target?.kind !== "external_daemon" || connectingRef.current) {
      return;
    }
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      const status = FIXTURE_MODE
        ? desktopRef.current
        : await removeExternalTarget(targetId);
      await acceptDesktopStatus(status, true);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleRestartManaged() {
    if (connectingRef.current) {
      return;
    }
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      if (!FIXTURE_MODE) {
        const status = await restartManagedRuntime();
        await acceptDesktopStatus(status, true);
      }
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleSetApprovalMode(approvalMode: ApprovalMode) {
    if (approvalModeChanging || submitting || connectingRef.current) {
      return;
    }
    setApprovalModeChanging(true);
    setActionError(null);
    try {
      const status = FIXTURE_MODE
        ? { ...desktopRef.current, approvalMode }
        : await setApprovalMode(approvalMode);
      desktopRef.current = status;
      setDesktop(status);
    } catch (error: unknown) {
      setActionError(commandError(error));
      await resyncDesktopAfterFailedMutation();
    } finally {
      setApprovalModeChanging(false);
    }
  }

  async function handleImportCaBundle() {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        return;
      }
      invalidateTargetRoute();
      const status = await importCaBundle();
      if (status !== null) {
        await acceptDesktopStatus(status, true);
      }
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleRemoveCaBundle() {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        return;
      }
      invalidateTargetRoute();
      await acceptDesktopStatus(await removeCaBundle(), true);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleSetTerminalEnabled(enabled: boolean) {
    const status = desktopRef.current;
    const selectedTarget = status.targets.find(
      (target) => target.targetId === status.selectedTargetId,
    );
    if (
      enabled &&
      selectedTarget?.terminalAvailable !== true &&
      !status.capabilities.shellTerminal
    ) {
      setSurface("settings");
      return;
    }
    try {
      const status = FIXTURE_MODE
        ? { ...desktopRef.current, terminalEnabled: enabled }
        : await setTerminalEnabled(enabled);
      desktopRef.current = status;
      setDesktop(status);
    } catch (error: unknown) {
      setActionError(commandError(error));
    }
  }

  async function handleOpenTerminal(
    kind: TerminalKind,
    planContext?: { sessionId: string; planId: string },
  ) {
    const status = desktopRef.current;
    const selectedTarget = status.targets.find(
      (target) => target.targetId === status.selectedTargetId,
    );
    const terminalAvailable =
      kind === "shell"
        ? status.capabilities.shellTerminal
        : selectedTarget?.terminalAvailable === true;
    if (!status.terminalEnabled || !terminalAvailable) {
      setSurface("settings");
      return;
    }
    try {
      if (!FIXTURE_MODE) {
        await showTerminalWindow(kind, planContext);
      }
    } catch (error: unknown) {
      setActionError(commandError(error));
    }
  }

  const activeView =
    chat.activeRunId === null ? undefined : chat.views.get(chat.activeRunId);
  const activeRun = activeView?.run;
  const conversationViews = useMemo(
    () => selectConversationViews(chat, activeRun?.sessionId ?? null),
    [activeRun?.sessionId, chat],
  );
  const openingRun = useMemo(() => {
    if (activeRun === undefined) {
      return undefined;
    }
    return chat.recentRuns
      .filter((run) => run.sessionId === activeRun.sessionId)
      .reduce<Run | undefined>(
        (opening, run) =>
          opening === undefined || run.createdAt < opening.createdAt
            ? run
            : opening,
        undefined,
      );
  }, [activeRun, chat.recentRuns]);
  const canCompose =
    connection.state === "connected" &&
    !connecting &&
    !submitting &&
    !approvalModeChanging &&
    (activeRun === undefined || isTerminalStatus(activeRun.status));
  const continuation =
    activeRun !== undefined && isTerminalStatus(activeRun.status);
  const promptBytes = utf8ByteLength(prompt);
  const promptOverLimit = promptBytes > MAX_PROMPT_BYTES;
  const views = useMemo(() => Array.from(chat.views.values()), [chat.views]);
  const selectedArtifacts = useMemo(
    () => selectReleasedArtifacts(conversationViews),
    [conversationViews],
  );
  const allArtifacts = useMemo(() => selectReleasedArtifacts(views), [views]);
  const activity = useMemo(() => selectOperationalActivity(views), [views]);
  const artifactItems = useMemo<ArtifactViewItem[]>(
    () =>
      selectedArtifacts.map((artifact) => {
        const artifactId = artifact.artifactId || artifact.key;
        const previewLines =
          previewFor(artifact.fileName) ?? artifactPreviews.get(artifactId);
        const previewError = artifactPreviewFailures.get(artifactId);
        return {
          id: artifactId,
          fileName: artifact.fileName,
          mediaType: artifact.mediaType,
          sizeLabel: artifact.sizeLabel,
          stateLabel: artifact.stateLabel,
          createdLabel: artifact.createdLabel,
          ...(previewLines === undefined ? {} : { previewLines }),
          previewStatus: artifactPreviewsLoading.has(artifactId)
            ? ("loading" as const)
            : previewError === undefined
              ? ("idle" as const)
              : ("error" as const),
          ...(previewError === undefined ? {} : { previewError }),
        };
      }),
    [
      artifactPreviewFailures,
      artifactPreviews,
      artifactPreviewsLoading,
      selectedArtifacts,
    ],
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
  const selectedTarget = desktop.targets.find(
    (target) => target.targetId === desktop.selectedTargetId,
  );
  const terminalAvailable = selectedTarget?.terminalAvailable === true;
  const workspaceFilesAvailable = desktop.capabilities.files;
  const workCount = new Set(chat.recentRuns.map((run) => run.sessionId)).size;
  const activeCount = new Set(
    chat.recentRuns
      .filter((run) =>
        ["queued", "running", "waiting", "cancelling"].includes(run.status),
      )
      .map((run) => run.sessionId),
  ).size;
  const title =
    activeRun === undefined
      ? "New work"
      : safeDisplayLabel(
          openingRun?.title ??
            conversationViews[0]?.run.title ??
            activeRun.title,
          agentRoleLabel(activeRun.role),
          160,
        );
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
      approvalMode={desktop.approvalMode}
      approvalModeVisible={selectedTarget?.kind === "managed_local"}
      approvalModeAvailable={
        selectedTarget?.kind === "managed_local" &&
        connection.state === "connected" &&
        (activeRun === undefined || isTerminalStatus(activeRun.status))
      }
      approvalModeChanging={approvalModeChanging}
      targetLabel={
        activeRun === undefined ? "Colossus" : agentRoleLabel(activeRun.role)
      }
      canCompose={canCompose}
      submitting={submitting}
      continuation={continuation}
      planRevision={
        planRevision === null
          ? null
          : {
              planId: planRevision.planId,
              revision: planRevision.revision,
            }
      }
      activeWorkRunning={
        activeRun !== undefined && !isTerminalStatus(activeRun.status)
      }
      activeWorkNeedsInput={(activeView?.pendingInteractions.length ?? 0) > 0}
      attachmentsAvailable={desktop.capabilities.attachments}
      attachments={attachments}
      attachmentBusy={attachmentBusy}
      error={composerError}
      onPromptChange={(nextPrompt) => {
        setPrompt(nextPrompt);
        setComposerError(null);
      }}
      onRoleChange={setRole}
      onMaxTurnsChange={(turns) => setMaxTurns(clampMaxTurns(turns))}
      onModeChange={(nextMode) => {
        if (planRevision === null) {
          setMode(nextMode);
        }
      }}
      onApprovalModeChange={(nextMode) => void handleSetApprovalMode(nextMode)}
      onCancelPlanRevision={() => {
        setPlanRevision(null);
        setComposerError(null);
      }}
      onChooseAttachment={() => void chooseAttachment()}
      onRemoveAttachment={(artifactId) =>
        setAttachments((current) =>
          current.filter((attachment) => attachment.artifactId !== artifactId),
        )
      }
      onSubmit={(event) => void submitRun(event)}
    />
  );
  const onboardingRequired = managedOnboardingRequired(desktop);
  const onboardingActive = showOnboarding || onboardingRequired;
  const developerPreview = releaseChannel === "developer_preview";

  return (
    <div
      className={`app-shell${developerPreview ? " app-shell--developer-preview" : ""}`}
    >
      <a className="skip-link" href="#primary-workspace">
        Skip to workspace
      </a>
      <ReleaseChannelBanner releaseChannel={releaseChannel} />
      <ProductRail
        surface={surface}
        attentionCount={attentionCount}
        connectionState={connection.state}
        terminalEnabled={desktop.terminalEnabled}
        terminalAvailable={terminalAvailable}
        capabilities={desktop.capabilities}
        onSelect={selectSurface}
        onOpenTerminal={() => void handleOpenTerminal("colossus_tui")}
        onOpenShell={() => void handleOpenTerminal("shell")}
      />

      {!onboardingActive && surface === "work" && workNavigationOpen ? (
        <button
          className="workspace-drawer-backdrop work-navigation-backdrop"
          type="button"
          aria-label="Close work navigation"
          aria-hidden="true"
          tabIndex={-1}
          onClick={closeWorkNavigation}
        />
      ) : null}

      {onboardingActive ? null : surface === "work" ? (
        <WorkSidebar
          runs={chat.recentRuns}
          workspace={desktop.workspace}
          activeSessionId={activeRun?.sessionId ?? null}
          query={workQuery}
          busy={listBusy}
          error={listError}
          hasMore={chat.nextPageToken !== ""}
          disabled={submitting || connecting}
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
          runCount={workCount}
          activeCount={activeCount}
          artifactCount={allArtifacts.length}
          activityCount={activity.length}
        />
      )}

      {onboardingActive ? (
        <OnboardingSurface
          desktop={desktop}
          busy={connecting}
          error={actionError?.message ?? ""}
          onChooseWorkspace={handleChooseWorkspace}
          onConfigure={handleConfigureManaged}
          onApplyConfiguration={handleApplyManagedModelConfiguration}
          onCodexLogin={handleCodexLogin}
          onCodexLogout={handleCodexLogout}
          onRunSelfTest={handleManagedSelfTest}
          onUseExternal={async () => {
            await handleAddExternalTarget();
          }}
          dismissible={showOnboarding && !onboardingRequired}
          onCancel={() => {
            setActionError(null);
            setShowOnboarding(false);
          }}
        />
      ) : surface === "work" ? (
        <WorkSurface
          title={title}
          view={activeView}
          conversationViews={conversationViews}
          connection={connection}
          connecting={connecting}
          cancelling={cancelling}
          runLoadError={runLoadError}
          actionError={actionError}
          participants={participants}
          artifacts={artifactItems}
          composer={composer}
          filesAvailable={desktop.capabilities.files}
          artifactsAvailable={desktop.capabilities.artifacts}
          planContinuationAvailable={desktop.capabilities.planContinuation}
          planWorkflowAvailable={
            desktop.terminalEnabled &&
            terminalAvailable &&
            desktop.capabilities.tui
          }
          filesPanel={
            <WorkspaceFiles
              workspace={desktop.workspace}
              available={workspaceFilesAvailable}
              listDirectory={
                FIXTURE_MODE
                  ? listFixtureWorkspaceDirectory
                  : listWorkspaceDirectory
              }
              readFile={
                FIXTURE_MODE ? readFixtureWorkspaceFile : readWorkspaceFile
              }
              onOpenSettings={() => setSurface("settings")}
            />
          }
          workNavigationOpen={workNavigationOpen}
          onConnect={() => void connect(desktop.selectedTargetId ?? undefined)}
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
          onSelectArtifact={(artifactId) =>
            void loadArtifactPreview(artifactId)
          }
          onOpenPlanWorkflow={(sessionId, planId) =>
            void handleOpenTerminal("colossus_tui", { sessionId, planId })
          }
          onRevisePlan={beginPlanRevision}
          onExecutePlan={executePlan}
          onOpenWorkNavigation={openWorkNavigation}
          onCloseWorkNavigation={closeWorkNavigation}
        />
      ) : (
        <OperationsSurface
          surface={surface}
          connection={connection}
          desktop={desktop}
          connecting={connecting}
          updateChecking={updateChecking}
          updateMessage={updateMessage}
          runs={chat.recentRuns}
          artifacts={allArtifacts}
          activity={activity}
          demoParticipants={FIXTURE_MODE ? DEMO_PARTICIPANTS : null}
          onConnect={() => void connect(desktop.selectedTargetId ?? undefined)}
          onOpenRun={(run) => void openRun(run)}
          onSelectTarget={(targetId) => void handleSelectTarget(targetId)}
          onAddExternalTarget={() => void handleAddExternalTarget()}
          onRemoveExternalTarget={(targetId) =>
            void handleRemoveExternalTarget(targetId)
          }
          onChooseWorkspace={() => void handleChooseWorkspace()}
          onConfigureManaged={() => setShowOnboarding(true)}
          onRestartManaged={() => void handleRestartManaged()}
          onSetTerminalEnabled={(enabled) =>
            void handleSetTerminalEnabled(enabled)
          }
          onOpenTerminal={(kind) => void handleOpenTerminal(kind)}
          onExportDiagnostics={() => {
            void exportDiagnostics().catch((error: unknown) => {
              setActionError(
                error instanceof CommandFailure
                  ? error.detail
                  : FALLBACK_ACTION_ERROR,
              );
            });
          }}
          onCheckForUpdates={() => void handleCheckDesktopUpdate()}
          onInstallUpdate={() => void handleInstallDesktopUpdate()}
          onImportCaBundle={() => void handleImportCaBundle()}
          onRemoveCaBundle={() => void handleRemoveCaBundle()}
        />
      )}
    </div>
  );
}
