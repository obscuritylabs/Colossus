import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";
import type { CSSProperties, FormEvent } from "react";

import {
  CommandFailure,
  addExternalTarget,
  applyManagedModelConfiguration,
  archiveThread,
  cancelRun,
  checkDesktopUpdate,
  chooseRunAttachment,
  chooseWorkspace,
  codexAuthLogin,
  codexAuthLogout,
  configureManagedRuntime,
  connectColossus,
  createSpace,
  createRun,
  desktopReleaseMetadata,
  desktopStatus,
  exportDiagnostics,
  getRun,
  getSessionMap,
  getThreadDelegate,
  importCaBundle,
  installDesktopUpdate,
  initializeDesktop,
  listAsides,
  listSessionActivity,
  listWorkspaceDirectory,
  listRuns,
  onSpaceAttention,
  onSpaceStatusChanged,
  readArtifactContent,
  readWorkspaceFile,
  renameSpace,
  restartManagedRuntime,
  restoreThread,
  restoreSpace,
  removeCaBundle,
  removeExternalTarget,
  respondInteraction,
  resolvePluginSelection,
  runManagedSelfTest,
  searchSpaceThreads,
  selectSpace,
  selectTarget,
  setApprovalMode,
  setTerminalEnabled,
  showTerminalWindow,
  archiveSpace,
  watchRun,
} from "./api";
import type { AgentParticipant } from "./components/AgentFlow";
import type {
  ArtifactPreviewLine,
  ArtifactViewItem,
} from "./components/ArtifactWorkspace";
import {
  ExecutionBoundaryBanner,
  executionBoundaryBannerVisible,
  managedRuntimeBoundaryActive,
} from "./components/ExecutionBoundaryBanner";
import { OperationsSurface } from "./components/OperationsSurface";
import { pluginSelectionKey } from "./plugins";
import { usePluginSkills } from "./use-plugin-skills";
import type { AsideDraft } from "./components/AsidePanel";
import { OnboardingSurface } from "./components/OnboardingSurface";
import { ReleaseChannelBanner } from "./components/ReleaseChannelBanner";
import type { WorkspaceSurface } from "./components/ProductRail";
import { WorkComposer } from "./components/WorkComposer";
import { WorkSidebar } from "./components/WorkSidebar";
import { ToastRegion, useToastQueue } from "./components/ToastRegion";
import type {
  SpaceActionFeedback,
  SpaceSearchScope,
  SpaceStartup,
} from "./components/WorkSidebar";
import { WorkSurface } from "./components/WorkSurface";
import type { SessionWorkspaceView } from "./components/SessionWorkspace";
import type { WorkspaceFileOpenRequest } from "./components/WorkspaceFiles";
import { WorkspaceFiles } from "./components/WorkspaceFiles";
import { managedOnboardingRequired } from "./onboarding";
import {
  parseDesktopSlashCommand,
  type DesktopSlashAction,
} from "./slash-commands";
import {
  enqueueMessage,
  messagesForThread,
  nextPendingMessage,
  removeQueuedMessage,
  updateQueuedMessage,
} from "./message-queue";
import type { QueuePlacement, QueuedMessage } from "./message-queue";
import {
  REMOTE_PROVIDER_TIMEOUT_MS,
  automaticProviderTimeoutMs,
} from "./providerTimeout";
import { selectSessionParticipants } from "./participants";
import {
  agentRoleLabel,
  safeDisplayLabel,
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
  withoutEntry,
} from "./state";
import type { IdempotentAttempt, RunView } from "./state";
import {
  clearStoredWorkSidebarWidth,
  readStoredWorkSidebarWidth,
  storeWorkSidebarWidth,
} from "./sidebar-width";
import { projectSpaceArchived, projectSpaceRestored } from "./space-lifecycle";
import {
  pinnedThreadIdsForSpace,
  readStoredThreadPins,
  setThreadPinned,
  storeThreadPins,
} from "./thread-pins";
import {
  readStoredThreadNames,
  setThreadName,
  storeThreadNames,
  threadNameForWorkspace,
} from "./thread-names";
import {
  TargetRouteRegistry,
  selectedTargetRouteChanged,
  watchDurableRun,
} from "./target-routing";
import type { TargetRoute } from "./target-routing";
import type {
  ApplyManagedModelConfigurationRequest,
  Aside,
  ApprovalMode,
  ArtifactReference,
  CommandError,
  ConfigureManagedRuntimeRequest,
  ConnectionStatus,
  CreateRunRequest,
  DesktopReleaseMetadata,
  DesktopStatus,
  Interaction,
  InteractionAnswer,
  ResearchDepth,
  ResearchSourceKind,
  Run,
  RunDetails,
  RunMode,
  RunStatus,
  RunUpdate,
  SessionMap,
  SpaceSearchResult,
  TerminalKind,
  ThreadDelegateInspection,
} from "./types";
import { USE_CONFIGURED_MAX_TURNS, isTerminalStatus } from "./types";
import {
  listFixtureWorkspaceDirectory,
  readFixtureWorkspaceFile,
} from "./dev/workspace-files-fixture";

const FIXTURE_QUERY = new URLSearchParams(window.location.search);
const FIXTURE_SCENARIO = FIXTURE_QUERY.get("fixture");
const FIXTURE_MODE =
  import.meta.env.DEV &&
  (FIXTURE_SCENARIO === "operations-studio" ||
    FIXTURE_SCENARIO === "activity-comparison" ||
    FIXTURE_SCENARIO === "interaction-question" ||
    FIXTURE_SCENARIO === "plan-workflow");
const FIXTURE_ACTIVITY_LIVE =
  FIXTURE_MODE && FIXTURE_QUERY.get("activityLive") === "1";
const FIXTURE_SPACE_STARTUP =
  FIXTURE_MODE && FIXTURE_QUERY.get("spaceStartup") === "1";
const FIXTURE_CONNECTION_STARTUP =
  FIXTURE_MODE &&
  (FIXTURE_SPACE_STARTUP || FIXTURE_QUERY.get("connecting") === "1");
const DEVELOPMENT_FIXTURES = import.meta.env.DEV
  ? await import("./dev/operations-studio-fixture")
  : null;

function developmentFixtures() {
  if (DEVELOPMENT_FIXTURES === null) {
    throw new Error("Development fixtures are unavailable in production.");
  }
  return DEVELOPMENT_FIXTURES;
}

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
  spaces: FIXTURE_MODE
    ? [
        {
          spaceId: "fixture-managed-local",
          targetId: "fixture-managed-local",
          displayName: "Colossus",
          displayPath: "~/tools/Colossus",
          archived: false,
          lastOpenedAtMs: Date.now(),
          lastActivityAt: null,
          state: "ready",
          message: "Fixture runtime ready.",
          selected: true,
          attentionCount: 1,
          providerConfigured: true,
        },
        {
          spaceId: "fixture-research",
          targetId: "fixture-research",
          displayName: "Research Lab",
          displayPath: "~/tools/research-lab",
          archived: false,
          lastOpenedAtMs: Date.now() - 1,
          lastActivityAt: "2026-07-20T14:20:00Z",
          state: "sleeping",
          message: "Starts when selected.",
          selected: false,
          attentionCount: 2,
          providerConfigured: true,
        },
        {
          spaceId: "fixture-proposal",
          targetId: "fixture-proposal",
          displayName: "Proposal Studio",
          displayPath: "~/tools/proposal-studio",
          archived: false,
          lastOpenedAtMs: Date.now() - 2,
          lastActivityAt: "2026-07-20T14:10:00Z",
          state: "sleeping",
          message: "Background work needs attention.",
          selected: false,
          attentionCount: 3,
          providerConfigured: true,
        },
        {
          spaceId: "fixture-personal",
          targetId: "fixture-personal",
          displayName: "Personal",
          displayPath: "~/personal",
          archived: false,
          lastOpenedAtMs: Date.now() - 3,
          lastActivityAt: null,
          state: "sleeping",
          message: "Starts when selected.",
          selected: false,
          attentionCount: 0,
          providerConfigured: true,
        },
      ]
    : [],
  selectedSpaceId: FIXTURE_MODE ? "fixture-managed-local" : null,
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
            capabilities: {
              toolCalls: true,
              streaming: true,
              imageInputs: false,
            },
          },
        ]
      : [],
    roles: FIXTURE_MODE ? { primary: "primary" } : {},
  },
  accessProfile: "allow_all",
  executionBoundary: "full_access",
  approvalMode: "ask",
  terminalEnabled: false,
  additionalCaBundle: {
    configured: false,
    certificateCount: 0,
    fingerprintsSha256: [],
  },
  capabilities: {
    research: true,
    delegation: false,
    plugins: false,
    pluginSkillSelection: FIXTURE_MODE,
    tui: FIXTURE_MODE,
    shellTerminal: FIXTURE_MODE,
    files: FIXTURE_MODE,
    artifacts: FIXTURE_MODE,
    planContinuation: FIXTURE_MODE,
    sessionActivity: FIXTURE_MODE,
    updateAvailable: false,
    agentWorkflows: false,
    attachments: false,
  },
};

const INITIAL_RELEASE_METADATA: DesktopReleaseMetadata = {
  platform: "unsupported",
  architecture: "unknown",
  channel: INITIAL_DESKTOP.releaseChannel,
  bundleIntegrity: "failed",
  codeSigning: "unsupported",
};

const FIXTURE_SPACE_THREAD_PREVIEWS: ReadonlyMap<
  string,
  readonly SpaceSearchResult[]
> = new Map([
  [
    "fixture-research",
    [
      {
        spaceId: "fixture-research",
        spaceName: "Research Lab",
        targetId: "fixture-research",
        runId: "fixture-research-source-review",
        sessionId: "fixture-research-source-review",
        title: "Review source provenance",
        mode: "research",
        status: "waiting",
        updatedAt: "2026-07-20T14:20:00Z",
        archived: false,
        threadArchived: false,
        attention: true,
      },
      {
        spaceId: "fixture-research",
        spaceName: "Research Lab",
        targetId: "fixture-research",
        runId: "fixture-research-search-backends",
        sessionId: "fixture-research-search-backends",
        title: "Compare search backends",
        mode: "research",
        status: "running",
        updatedAt: "2026-07-20T14:16:00Z",
        archived: false,
        threadArchived: false,
        attention: false,
      },
    ],
  ],
  [
    "fixture-proposal",
    [
      {
        spaceId: "fixture-proposal",
        spaceName: "Proposal Studio",
        targetId: "fixture-proposal",
        runId: "fixture-proposal-compliance",
        sessionId: "fixture-proposal-compliance",
        title: "Resolve compliance findings",
        mode: "execute",
        status: "waiting",
        updatedAt: "2026-07-20T14:10:00Z",
        archived: false,
        threadArchived: false,
        attention: true,
      },
      {
        spaceId: "fixture-proposal",
        spaceName: "Proposal Studio",
        targetId: "fixture-proposal",
        runId: "fixture-proposal-pricing",
        sessionId: "fixture-proposal-pricing",
        title: "Polish pricing narrative",
        mode: "execute",
        status: "completed",
        updatedAt: "2026-07-20T13:48:00Z",
        archived: false,
        threadArchived: false,
        attention: false,
      },
    ],
  ],
  [
    "fixture-personal",
    [
      {
        spaceId: "fixture-personal",
        spaceName: "Personal",
        targetId: "fixture-personal",
        runId: "fixture-personal-weekly-plan",
        sessionId: "fixture-personal-weekly-plan",
        title: "Draft weekly plan",
        mode: "plan",
        status: "completed",
        updatedAt: "2026-07-19T18:30:00Z",
        archived: false,
        threadArchived: false,
        attention: false,
      },
    ],
  ],
]);

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
    kind: "primary",
  },
  {
    id: "builder",
    name: "Builder",
    role: "Engineer",
    state: "working",
    icon: "builder",
    kind: "delegate",
    parentRunId: "fixture-run-desktop-release",
    childSessionId: "fixture-session-builder",
    childRunId: "fixture-run-builder",
    parentRunIndex: 1,
    parentRunTitle: "Harden desktop agent bootstrap",
    modelRole: "builder",
    task: "Implement the approved desktop changes.",
  },
  {
    id: "sentinel",
    name: "Sentinel",
    role: "Security",
    state: "reviewing",
    icon: "security",
    kind: "delegate",
    parentRunId: "fixture-run-desktop-release",
    childSessionId: "fixture-session-sentinel",
    childRunId: "fixture-run-sentinel",
    parentRunIndex: 1,
    parentRunTitle: "Harden desktop agent bootstrap",
    modelRole: "security_reviewer",
    task: "Conduct a read-only security review of process and session boundaries.",
  },
  {
    id: "scribe",
    name: "Scribe",
    role: "Writer",
    state: "waiting",
    icon: "writer",
    kind: "delegate",
    parentRunId: "fixture-run-desktop-release",
    childSessionId: "fixture-session-scribe",
    childRunId: "fixture-run-scribe",
    parentRunIndex: 1,
    parentRunTitle: "Harden desktop agent bootstrap",
    modelRole: "writer",
    task: "Summarize the released implementation and review findings.",
  },
];

function buildDelegateFixture(
  participant: AgentParticipant,
  parent: Run,
): { details: RunDetails; updates: readonly RunUpdate[] } {
  const runId = participant.childRunId ?? `fixture-run-${participant.id}`;
  const sessionId =
    participant.childSessionId ?? `fixture-session-${participant.id}`;
  const startedAt = "2026-07-20T14:35:10Z";
  const finishedAt = "2026-07-20T14:35:24Z";
  const output =
    "No cross-workspace control path was found. Session ownership remains bound to the selected Workspace, and the reviewed process boundary does not expose another Workspace's run authority.";
  const run: Run = {
    ...parent,
    runId,
    sessionId,
    title: participant.task ?? participant.role,
    role: participant.modelRole ?? "subagent_default",
    status: "completed",
    createdAt: startedAt,
    updatedAt: finishedAt,
    startedAt,
    finishedAt,
    lastSequence: 8,
    pendingInteractionCount: 0,
    terminal: {
      type: "result",
      result: {
        output,
        profile: "desktop",
        modelProfile: "delegated",
        providerProfile: "fixture-provider",
        model: "openrouter/auto",
        elapsedSeconds: 14,
      },
    },
    etag: `fixture-etag-${runId}`,
    archived: false,
  };
  const activities: Array<{
    sequence: number;
    createdAt: string;
    callId: string;
    toolName: string;
    state: "started" | "completed";
    input?: string;
    preview?: string;
  }> = [
    {
      sequence: 1,
      createdAt: "2026-07-20T14:35:11.000Z",
      callId: "delegate-policy",
      toolName: "filesystem.read",
      state: "started",
      input: '{"path":"docs/develop/security-architecture.md"}',
    },
    {
      sequence: 2,
      createdAt: "2026-07-20T14:35:12.100Z",
      callId: "delegate-policy",
      toolName: "filesystem.read",
      state: "completed",
      preview: "Released security-boundary documentation.",
    },
    {
      sequence: 3,
      createdAt: "2026-07-20T14:35:13.000Z",
      callId: "delegate-session",
      toolName: "repo.search",
      state: "started",
      input: '{"query":"selected Workspace session boundary"}',
    },
    {
      sequence: 4,
      createdAt: "2026-07-20T14:35:16.200Z",
      callId: "delegate-session",
      toolName: "repo.search",
      state: "completed",
      preview: "Matched selected-Workspace routing and ownership checks.",
    },
    {
      sequence: 5,
      createdAt: "2026-07-20T14:35:17.000Z",
      callId: "delegate-dependencies",
      toolName: "repo.file_summary",
      state: "started",
      input: '{"path":"apps/desktop/src-tauri/src/commands.rs"}',
    },
    {
      sequence: 6,
      createdAt: "2026-07-20T14:35:22.800Z",
      callId: "delegate-dependencies",
      toolName: "repo.file_summary",
      state: "completed",
      preview: "Renderer requests remain bound to the selected target.",
    },
  ];
  const updates: RunUpdate[] = activities.map((activity) => ({
    runId,
    sequence: activity.sequence,
    createdAt: activity.createdAt,
    update: {
      type: "tool_activity",
      activity: {
        callId: activity.callId,
        toolName: activity.toolName,
        state: activity.state,
        summary: `tool execution ${activity.state}`,
        ...(activity.input === undefined ? {} : { input: activity.input }),
        ...(activity.preview === undefined
          ? {}
          : { preview: activity.preview }),
      },
    },
  }));
  return { details: { run, pendingInteractions: [] }, updates };
}

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

interface RunSubmission {
  prompt: string;
  executionPrompt?: string;
  pluginSkillIds?: readonly string[];
  attachments: readonly ArtifactReference[];
  role: string;
  mode: RunMode;
  researchDepth: ResearchDepth;
  researchSources: readonly ResearchSourceKind[];
  maxTurns: number;
  idempotencyKey: string;
  sessionId?: string;
  planRevision?: PlanRevisionTarget;
}

type RunSubmissionResult =
  | { type: "accepted"; run: Run }
  | { type: "failed"; error: CommandError }
  | { type: "stale" };

export default function App() {
  const [chat, dispatch] = useReducer(
    chatReducer,
    FIXTURE_MODE
      ? FIXTURE_SCENARIO === "activity-comparison"
        ? developmentFixtures().buildActivityComparisonFixture()
        : FIXTURE_SCENARIO === "plan-workflow"
          ? developmentFixtures().buildPlanWorkflowFixture()
          : developmentFixtures().buildOperationsStudioFixture(
              FIXTURE_SCENARIO === "interaction-question"
                ? "user_prompt"
                : "approval",
            )
      : initialChatState,
  );
  const chatRef = useRef(chat);
  const [asideChat, dispatchAside] = useReducer(chatReducer, initialChatState);
  const [delegateChat, dispatchDelegate] = useReducer(
    chatReducer,
    initialChatState,
  );
  const [selectedDelegateId, setSelectedDelegateId] = useState<string | null>(
    null,
  );
  const [selectedDelegateRunId, setSelectedDelegateRunId] = useState<
    string | null
  >(null);
  const [delegateLoading, setDelegateLoading] = useState(false);
  const [delegateError, setDelegateError] = useState("");
  const [delegateInspection, setDelegateInspection] =
    useState<ThreadDelegateInspection | null>(null);
  const [sessionMap, setSessionMap] = useState<SessionMap | null>(() =>
    FIXTURE_MODE ? developmentFixtures().buildSessionMapFixture() : null,
  );
  const [sessionMapLoading, setSessionMapLoading] = useState(false);
  const [sessionMapError, setSessionMapError] = useState("");
  const sessionMapRequest = useRef<symbol | null>(null);
  const [activeSessionWorkspaceView, setActiveSessionWorkspaceView] =
    useState<SessionWorkspaceView>(
      FIXTURE_QUERY.get("view") === "activity" ? "activity" : "conversation",
    );
  const [asideHistory, setAsideHistory] = useState<readonly Aside[]>([]);
  const [asideBusy, setAsideBusy] = useState(false);
  const [asideError, setAsideError] = useState<CommandError | null>(null);
  const [asideReadOnly, setAsideReadOnly] = useState(false);
  const appShellRef = useRef<HTMLDivElement>(null);
  const [initialWorkSidebarWidth] = useState(readStoredWorkSidebarWidth);
  const workSidebarWidthRef = useRef(initialWorkSidebarWidth);
  const [desktop, setDesktop] = useState<DesktopStatus>(INITIAL_DESKTOP);
  const [conversationSkills, setConversationSkills] = useState<
    Record<string, readonly string[]>
  >({});
  const selectionSession =
    chat.activeRunId === null
      ? undefined
      : chat.views.get(chat.activeRunId)?.run.sessionId;
  const selectionKey = pluginSelectionKey(
    desktop.selectedTargetId,
    selectionSession,
  );
  const pluginSelections = conversationSkills[selectionKey] ?? [];
  const [storedThreadPins, setStoredThreadPins] = useState(() => {
    const fixtureSessionId =
      FIXTURE_MODE && chat.activeRunId !== null
        ? chat.views.get(chat.activeRunId)?.run.sessionId
        : undefined;
    const fixtureDefaults =
      desktop.selectedSpaceId !== null && fixtureSessionId !== undefined
        ? setThreadPinned([], desktop.selectedSpaceId, fixtureSessionId, true)
        : [];
    return readStoredThreadPins(fixtureDefaults);
  });
  const pinnedThreadSessionIds = useMemo(
    () =>
      new Set(
        pinnedThreadIdsForSpace(storedThreadPins, desktop.selectedSpaceId),
      ),
    [desktop.selectedSpaceId, storedThreadPins],
  );
  const [storedThreadNames, setStoredThreadNames] = useState(
    readStoredThreadNames,
  );
  const resolveThreadTitle = useCallback(
    (spaceId: string | null, sessionId: string, fallback: string) =>
      threadNameForWorkspace(storedThreadNames, spaceId, sessionId) ?? fallback,
    [storedThreadNames],
  );
  const [releaseChannel, setReleaseChannel] = useState(
    INITIAL_DESKTOP.releaseChannel,
  );
  const [releaseMetadata, setReleaseMetadata] =
    useState<DesktopReleaseMetadata>(INITIAL_RELEASE_METADATA);
  const desktopRef = useRef(desktop);
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [surface, setSurface] = useState<WorkspaceSurface>("work");
  const [workNavigationOpen, setWorkNavigationOpen] = useState(false);
  const [workspaceFileOpenRequest, setWorkspaceFileOpenRequest] =
    useState<WorkspaceFileOpenRequest | null>(null);
  const workspaceFileOpenSequence = useRef(0);
  const fixtureActivityStartedAt = useRef<number | null>(null);
  const [spaceStartup, setSpaceStartup] = useState<SpaceStartup | null>(() =>
    FIXTURE_SPACE_STARTUP
      ? { spaceId: "fixture-research", displayName: "Research Lab" }
      : null,
  );
  const [workQuery, setWorkQuery] = useState("");
  const deferredWorkQuery = useDeferredValue(workQuery);
  const [searchScope, setSearchScope] = useState<SpaceSearchScope>("space");
  const [includeArchivedSearch, setIncludeArchivedSearch] = useState(false);
  const [spaceSearchResults, setSpaceSearchResults] = useState<
    readonly SpaceSearchResult[]
  >([]);
  const [spaceSearchCursor, setSpaceSearchCursor] = useState("");
  const [spaceSearchBusy, setSpaceSearchBusy] = useState(false);
  const [spaceSearchError, setSpaceSearchError] = useState("");
  const [spaceThreadPreviews, setSpaceThreadPreviews] = useState<
    ReadonlyMap<string, readonly SpaceSearchResult[]>
  >(new Map());
  const [spaceThreadPreviewBusyIds, setSpaceThreadPreviewBusyIds] = useState<
    ReadonlySet<string>
  >(new Set());
  const [spaceThreadPreviewErrors, setSpaceThreadPreviewErrors] = useState<
    ReadonlyMap<string, string>
  >(new Map());
  const spaceThreadPreviewRequests = useRef(new Set<string>());
  const connection = desktop.connection;
  const [connecting, setConnecting] = useState(
    !FIXTURE_MODE || FIXTURE_CONNECTION_STARTUP,
  );
  const [listBusy, setListBusy] = useState(false);
  const [listError, setListError] = useState("");
  const [runLoadError, setRunLoadError] = useState("");
  const [prompt, setPrompt] = useState("");
  const completionSkills = usePluginSkills(
    desktop.selectedTargetId,
    desktop.capabilities.pluginSkillSelection === true,
    prompt.trimStart().startsWith("@") || pluginSelections.length > 0,
  );
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
  const [researchDepth, setResearchDepth] = useState<ResearchDepth>("standard");
  const [researchSources, setResearchSources] = useState<ResearchSourceKind[]>([
    "repo",
  ]);
  const [planRevision, setPlanRevision] = useState<PlanRevisionTarget | null>(
    null,
  );
  const [maxTurns, setMaxTurns] = useState(USE_CONFIGURED_MAX_TURNS);
  const [submitting, setSubmitting] = useState(false);
  const [approvalModeChanging, setApprovalModeChanging] = useState(false);
  const [composerError, setComposerError] = useState<CommandError | null>(null);
  const { dismissToast, pushToast, toasts } = useToastQueue();
  const [conversationFollowRequest, setConversationFollowRequest] = useState(0);
  const [queuedMessages, setQueuedMessages] = useState<
    readonly QueuedMessage[]
  >([]);
  const [actionError, setActionError] = useState<CommandError | null>(null);
  const [spaceActionFeedback, setSpaceActionFeedback] =
    useState<SpaceActionFeedback | null>(null);
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");
  const [cancelling, setCancelling] = useState(false);
  const [threadLifecycleBusySessionId, setThreadLifecycleBusySessionId] =
    useState<string | null>(null);
  const watchedRuns = useRef(new Map<string, symbol>());
  const watchedAsides = useRef(new Map<string, symbol>());
  const delegateRequest = useRef<symbol | null>(null);
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
  const queuedMessagesRef = useRef<readonly QueuedMessage[]>([]);
  const queueDeliveryRef = useRef<string | null>(null);
  const createAttempt = useRef<RoutedAttempt | null>(null);
  const asideCreateAttempt = useRef<RoutedAttempt | null>(null);
  const cancelAttempts = useRef(new Map<string, IdempotentAttempt>());
  const responseAttempts = useRef(new Map<string, IdempotentAttempt>());
  const threadLifecycleAttempts = useRef(new Map<string, IdempotentAttempt>());
  const listRequest = useRef<symbol | null>(null);
  const cancelRequest = useRef<symbol | null>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const composerFormRef = useRef<HTMLFormElement>(null);

  const commitQueuedMessages = useCallback(
    (messages: readonly QueuedMessage[]) => {
      queuedMessagesRef.current = messages;
      setQueuedMessages(messages);
    },
    [],
  );

  const previewWorkSidebarWidth = useCallback((width: number) => {
    workSidebarWidthRef.current = width;
    appShellRef.current?.style.setProperty(
      "--work-sidebar-width",
      `${width}px`,
    );
  }, []);

  const commitWorkSidebarWidth = useCallback(
    (width: number) => {
      previewWorkSidebarWidth(width);
      storeWorkSidebarWidth(width);
    },
    [previewWorkSidebarWidth],
  );

  const resetWorkSidebarWidth = useCallback(() => {
    workSidebarWidthRef.current = null;
    appShellRef.current?.style.removeProperty("--work-sidebar-width");
    clearStoredWorkSidebarWidth();
  }, []);

  const updateThreadPin = useCallback(
    (spaceId: string, sessionId: string, pinned: boolean) => {
      setStoredThreadPins((current) => {
        const next = setThreadPinned(current, spaceId, sessionId, pinned);
        storeThreadPins(next);
        return next;
      });
    },
    [],
  );

  function handleToggleThreadPinned(run: Run) {
    const spaceId = desktopRef.current.selectedSpaceId;
    if (spaceId === null) {
      return;
    }
    updateThreadPin(
      spaceId,
      run.sessionId,
      !pinnedThreadSessionIds.has(run.sessionId),
    );
  }

  function handleRenameThread(run: Run, name: string) {
    const spaceId = desktopRef.current.selectedSpaceId;
    if (spaceId === null) {
      return;
    }
    setStoredThreadNames((current) => {
      const next = setThreadName(current, spaceId, run.sessionId, name);
      storeThreadNames(next);
      return next;
    });
  }

  useEffect(() => {
    chatRef.current = chat;
  }, [chat]);

  useEffect(() => {
    desktopRef.current = desktop;
  }, [desktop]);

  useEffect(() => {
    dispatchAside({ type: "reset" });
    setAsideHistory([]);
    setAsideError(null);
    setAsideReadOnly(false);
    asideCreateAttempt.current = null;
  }, [desktop.selectedTargetId]);

  useEffect(() => {
    if (FIXTURE_MODE) {
      return;
    }
    let cancelled = false;
    let unlistenStatus: (() => void) | undefined;
    let unlistenAttention: (() => void) | undefined;
    void Promise.all([
      onSpaceStatusChanged((summary) => {
        if (cancelled) {
          return;
        }
        setDesktop((current) => ({
          ...current,
          spaces: current.spaces.map((space) =>
            space.spaceId === summary.spaceId
              ? {
                  ...space,
                  displayName: summary.displayName,
                  archived: summary.archived,
                  state: summary.state,
                  selected: summary.selected,
                  attentionCount: summary.attentionCount,
                  lastActivityAt: summary.lastActivityAt,
                }
              : space,
          ),
        }));
      }),
      onSpaceAttention((attention) => {
        if (cancelled) {
          return;
        }
        setDesktop((current) => ({
          ...current,
          spaces: current.spaces.map((space) =>
            space.spaceId === attention.spaceId
              ? { ...space, attentionCount: attention.attentionCount }
              : space,
          ),
        }));
      }),
    ])
      .then(([stopStatus, stopAttention]) => {
        if (cancelled) {
          stopStatus();
          stopAttention();
        } else {
          unlistenStatus = stopStatus;
          unlistenAttention = stopAttention;
        }
      })
      .catch(() => {
        // The five-second native status refresh remains the fallback when the
        // WebView event bridge is unavailable during startup or teardown.
      });
    return () => {
      cancelled = true;
      unlistenStatus?.();
      unlistenAttention?.();
    };
  }, []);

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

  const startAsideWatch = useCallback(
    (runId: string, afterSequence: number, route: TargetRoute) => {
      if (FIXTURE_MODE || targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const watchKey = `${route.targetId}:${route.generation}:aside:${runId}`;
      if (watchedAsides.current.has(watchKey)) {
        return;
      }
      const token = Symbol(runId);
      watchedAsides.current.set(watchKey, token);
      dispatchAside({ type: "watch_started", runId });
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
        onUpdate: (update) => dispatchAside({ type: "ingest_update", update }),
        onHydrate: (details) => {
          targetRoutes.current?.bindRun(details.run.runId, route);
          dispatchAside({ type: "hydrate_run", details });
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
            dispatchAside({ type: "watch_complete", runId });
          } else {
            dispatchAside({ type: "watch_error", runId, error: result.error });
          }
        })
        .finally(() => {
          if (watchedAsides.current.get(watchKey) === token) {
            watchedAsides.current.delete(watchKey);
          }
        });
    },
    [],
  );

  const acceptDesktopStatus = useCallback(
    async (status: DesktopStatus, resetWork: boolean) => {
      const previousStatus = desktopRef.current;
      desktopRef.current = status;
      setDesktop(status);
      setReleaseChannel(status.releaseChannel);
      setReleaseMetadata((current) => ({
        ...current,
        channel: status.releaseChannel,
      }));
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
      setConnecting(FIXTURE_CONNECTION_STARTUP);
      return;
    }
    let cancelled = false;
    connectingRef.current = true;
    setConnecting(true);
    void desktopReleaseMetadata()
      .then((metadata) => {
        if (!cancelled) {
          setReleaseMetadata(metadata);
          setReleaseChannel(metadata.channel);
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

  useEffect(() => {
    const query = deferredWorkQuery.trim();
    if (query === "") {
      setSpaceSearchResults([]);
      setSpaceSearchCursor("");
      setSpaceSearchBusy(false);
      setSpaceSearchError("");
      return;
    }
    if (searchScope === "space" && desktop.selectedSpaceId === null) {
      setSpaceSearchResults([]);
      setSpaceSearchCursor("");
      setSpaceSearchBusy(false);
      setSpaceSearchError("Select a Workspace before searching it.");
      return;
    }

    let cancelled = false;
    const timer = window.setTimeout(() => {
      setSpaceSearchBusy(true);
      setSpaceSearchError("");
      if (FIXTURE_MODE) {
        const normalized = query.toLocaleLowerCase();
        const space = desktop.spaces.find(
          (candidate) => candidate.spaceId === desktop.selectedSpaceId,
        );
        const results = chat.recentRuns
          .filter((run) =>
            [run.title, run.mode, run.status]
              .join(" ")
              .toLocaleLowerCase()
              .includes(normalized),
          )
          .slice(0, 50)
          .map((run): SpaceSearchResult => ({
            spaceId: space?.spaceId ?? "fixture-managed-local",
            spaceName: space?.displayName ?? "Colossus",
            targetId: space?.targetId ?? "fixture-managed-local",
            runId: run.runId,
            sessionId: run.sessionId,
            title: run.title,
            mode: run.mode,
            status: run.status,
            updatedAt: run.updatedAt,
            archived: false,
            threadArchived: run.archived,
            attention:
              run.status === "waiting" ||
              run.status === "outcome_unknown" ||
              run.pendingInteractionCount > 0,
          }));
        if (!cancelled) {
          setSpaceSearchResults(results);
          setSpaceSearchCursor("");
          setSpaceSearchBusy(false);
        }
        return;
      }

      void searchSpaceThreads({
        query,
        ...(searchScope === "space" && desktop.selectedSpaceId !== null
          ? { spaceId: desktop.selectedSpaceId }
          : {}),
        includeArchived: searchScope === "all" && includeArchivedSearch,
        pageSize: 50,
      })
        .then((page) => {
          if (!cancelled) {
            setSpaceSearchResults(page.results);
            setSpaceSearchCursor(page.nextCursor);
          }
        })
        .catch((error: unknown) => {
          if (!cancelled) {
            setSpaceSearchResults([]);
            setSpaceSearchCursor("");
            setSpaceSearchError(commandError(error).message);
          }
        })
        .finally(() => {
          if (!cancelled) {
            setSpaceSearchBusy(false);
          }
        });
    }, 160);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [
    chat.recentRuns,
    deferredWorkQuery,
    desktop.selectedSpaceId,
    desktop.spaces,
    includeArchivedSearch,
    searchScope,
  ]);

  async function loadMoreSpaceSearch() {
    const query = deferredWorkQuery.trim();
    if (
      FIXTURE_MODE ||
      query === "" ||
      spaceSearchCursor === "" ||
      spaceSearchBusy
    ) {
      return;
    }
    setSpaceSearchBusy(true);
    setSpaceSearchError("");
    try {
      const page = await searchSpaceThreads({
        query,
        ...(searchScope === "space" && desktop.selectedSpaceId !== null
          ? { spaceId: desktop.selectedSpaceId }
          : {}),
        includeArchived: searchScope === "all" && includeArchivedSearch,
        cursor: spaceSearchCursor,
        pageSize: 50,
      });
      setSpaceSearchResults((current) => [...current, ...page.results]);
      setSpaceSearchCursor(page.nextCursor);
    } catch (error: unknown) {
      setSpaceSearchError(commandError(error).message);
    } finally {
      setSpaceSearchBusy(false);
    }
  }

  async function loadSpaceThreadPreview(spaceId: string) {
    if (
      spaceThreadPreviews.has(spaceId) ||
      spaceThreadPreviewRequests.current.has(spaceId)
    ) {
      return;
    }
    const space = desktopRef.current.spaces.find(
      (candidate) => candidate.spaceId === spaceId && !candidate.archived,
    );
    if (space === undefined) {
      return;
    }

    spaceThreadPreviewRequests.current.add(spaceId);
    setSpaceThreadPreviewBusyIds((current) => {
      const next = new Set(current);
      next.add(spaceId);
      return next;
    });
    setSpaceThreadPreviewErrors((current) => {
      const next = new Map(current);
      next.delete(spaceId);
      return next;
    });
    try {
      const results = FIXTURE_MODE
        ? (FIXTURE_SPACE_THREAD_PREVIEWS.get(spaceId) ??
          chatRef.current.recentRuns
            .slice(0, 8)
            .map((run): SpaceSearchResult => ({
              spaceId,
              spaceName: space.displayName,
              targetId: space.targetId,
              runId: run.runId,
              sessionId: run.sessionId,
              title: run.title,
              mode: run.mode,
              status: run.status,
              updatedAt: run.updatedAt,
              archived: false,
              threadArchived: run.archived,
              attention:
                run.status === "waiting" ||
                run.status === "outcome_unknown" ||
                run.pendingInteractionCount > 0,
            })))
        : (
            await searchSpaceThreads({
              query: "",
              spaceId,
              includeArchived: false,
              pageSize: 8,
            })
          ).results;
      setSpaceThreadPreviews((current) => {
        const next = new Map(current);
        next.set(spaceId, results);
        return next;
      });
    } catch (error: unknown) {
      setSpaceThreadPreviewErrors((current) => {
        const next = new Map(current);
        next.set(spaceId, commandError(error).message);
        return next;
      });
    } finally {
      spaceThreadPreviewRequests.current.delete(spaceId);
      setSpaceThreadPreviewBusyIds((current) => {
        const next = new Set(current);
        next.delete(spaceId);
        return next;
      });
    }
  }

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
    setConversationSkills((current) => ({
      ...current,
      [pluginSelectionKey(desktop.selectedTargetId, undefined)]: [],
    }));
    setSurface("work");
    dispatch({ type: "select_run", runId: null });
    setRunLoadError("");
    setActionError(null);
    setComposerError(null);
    setPlanRevision(null);
    setAttachments([]);
    requestAnimationFrame(() => composerRef.current?.focus());
  }

  const performRunSubmission = useCallback(
    async (
      submission: RunSubmission,
      route: TargetRoute,
    ): Promise<RunSubmissionResult> => {
      if (
        submitInFlight.current ||
        connectingRef.current ||
        targetRoutes.current?.isCurrent(route) !== true
      ) {
        return { type: "stale" };
      }
      const request: CreateRunRequest = {
        prompt: submission.executionPrompt ?? submission.prompt,
        ...(submission.executionPrompt === undefined
          ? {}
          : { pluginMentionsResolved: true }),
        pluginSkillIds: [...(submission.pluginSkillIds ?? [])],
        artifactIds: submission.attachments.map(
          (attachment) => attachment.artifactId,
        ),
        role: submission.role,
        mode: submission.mode,
        ...(submission.mode === "research"
          ? {
              researchDepth: submission.researchDepth,
              researchSources: [...submission.researchSources],
            }
          : {}),
        maxTurns: submission.maxTurns,
        idempotencyKey: submission.idempotencyKey,
        ...(submission.sessionId === undefined
          ? {}
          : { sessionId: submission.sessionId }),
        ...(submission.planRevision === undefined
          ? {}
          : {
              planAction: {
                type: "revise" as const,
                sourceRunId: submission.planRevision.sourceRunId,
                expectedRevision: submission.planRevision.revision,
              },
            }),
      };

      submitInFlight.current = true;
      setSubmitting(true);
      setActionError(null);
      try {
        let run: Run;
        if (FIXTURE_MODE) {
          const now = new Date().toISOString();
          const identity = crypto.randomUUID();
          const runId = `fixture-composed-${identity}`;
          run = {
            runId,
            sessionId: submission.sessionId ?? `fixture-session-${identity}`,
            title: safeDisplayLabel(submission.prompt, "Untitled work", 80),
            role: submission.role,
            mode: submission.mode,
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
                  submission.planRevision === undefined
                    ? "Showcase response: the request was accepted by the local Operations Studio fixture. Live builds send this through the scoped native command boundary."
                    : "The selected Plan was revised in this chat and saved as a new durable draft revision.",
                ...(submission.planRevision === undefined
                  ? {}
                  : {
                      planId: submission.planRevision.planId,
                      planRevision: submission.planRevision.revision + 1,
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
            archived: false,
          };
        } else {
          run = await createRun(route.targetId, request);
        }
        if (targetRoutes.current?.isCurrent(route) !== true) {
          return { type: "stale" };
        }
        targetRoutes.current.bindRun(run.runId, route);
        if (submission.sessionId === undefined) {
          setConversationSkills((current) => ({
            ...current,
            [pluginSelectionKey(route.targetId, run.sessionId)]:
              submission.pluginSkillIds ?? [],
          }));
        }
        dispatch({ type: "upsert_run", run });
        dispatch({
          type: "record_local_prompt",
          runId: run.runId,
          prompt: submission.prompt,
        });
        dispatch({ type: "select_run", runId: run.runId });
        if (!FIXTURE_MODE) {
          startWatch(run.runId, 0, route);
        }
        return { type: "accepted", run };
      } catch (error: unknown) {
        if (targetRoutes.current?.isCurrent(route) !== true) {
          return { type: "stale" };
        }
        const failure = commandError(error);
        markConnectionFailure(failure, route);
        return { type: "failed", error: failure };
      } finally {
        submitInFlight.current = false;
        setSubmitting(false);
      }
    },
    [markConnectionFailure, startWatch],
  );

  async function enqueueCurrentMessage(
    currentView: NonNullable<ReturnType<typeof chat.views.get>>,
    route: TargetRoute,
    placement: QueuePlacement,
  ): Promise<QueuedMessage | null> {
    const cleanPrompt = prompt.trim();
    const cleanRole = role.trim();
    if (
      cleanPrompt.length === 0 ||
      cleanRole.length === 0 ||
      !isPromptWithinByteLimit(prompt) ||
      (mode === "research" && researchSources.length === 0)
    ) {
      return null;
    }
    let resolved: { prompt: string; pluginSkillIds: string[] } | undefined;
    if (!FIXTURE_MODE && cleanPrompt.startsWith("@")) {
      if (submitInFlight.current) return null;
      submitInFlight.current = true;
      setSubmitting(true);
      try {
        resolved = await resolvePluginSelection(
          route.targetId,
          cleanPrompt,
          pluginSelections,
        );
        if (targetRoutes.current?.isCurrent(route) !== true) return null;
      } catch (error: unknown) {
        setComposerError(commandError(error));
        return null;
      } finally {
        submitInFlight.current = false;
        setSubmitting(false);
      }
    }
    const message: QueuedMessage = {
      id: crypto.randomUUID(),
      idempotencyKey: crypto.randomUUID(),
      targetId: route.targetId,
      sessionId: currentView.run.sessionId,
      prompt: cleanPrompt,
      ...(resolved === undefined ? {} : { executionPrompt: resolved.prompt }),
      pluginSkillIds: resolved?.pluginSkillIds ?? [...pluginSelections],
      conversationSkillIds: [...pluginSelections],
      role: cleanRole,
      mode,
      researchDepth,
      researchSources: [...researchSources],
      maxTurns,
      attachments: [...attachments],
      createdAt: new Date().toISOString(),
      state: "pending",
      error: null,
    };
    const result = enqueueMessage(
      queuedMessagesRef.current,
      message,
      placement,
    );
    if (!result.accepted) {
      setComposerError({
        ...FALLBACK_ACTION_ERROR,
        code: "message_queue_full",
        message:
          result.reason === "thread_full"
            ? "Next up is full for this thread. Edit or delete a queued message first."
            : "The Desktop message queue is full. Send or remove another queued message first.",
        retryable: false,
      });
      return null;
    }
    commitQueuedMessages(result.messages);
    setPrompt("");
    setAttachments([]);
    setComposerError(null);
    return message;
  }

  function setSlashCommandError(message: string) {
    setComposerError({
      code: "invalid_slash_command",
      message,
      retryable: false,
      outcomeUnknown: false,
      violations: [],
    });
  }

  async function handleDesktopSlashAction(
    action: DesktopSlashAction,
  ): Promise<"clear" | "preserve"> {
    setComposerError(null);
    switch (action.type) {
      case "show_help":
        setPrompt("/");
        pushToast("Choose a supported Desktop command.", "info");
        requestAnimationFrame(() => composerRef.current?.focus());
        return "preserve";
      case "new_work":
        newWork();
        pushToast("New work is ready.", "info");
        return "clear";
      case "open_work_navigation":
        setSurface("work");
        setWorkNavigationOpen(true);
        return "clear";
      case "set_mode":
        if (action.mode === "research" && !desktop.capabilities.research) {
          setSlashCommandError("Research is unavailable for this target.");
          return "preserve";
        }
        if (action.resetPlanRevision) {
          setPlanRevision(null);
        }
        setMode(action.mode);
        pushToast(
          `${action.mode === "plan" ? "Plan" : action.mode === "research" ? "Research" : "Execute"} mode enabled.`,
          "info",
        );
        return "clear";
      case "toggle_mode": {
        const nextMode: RunMode =
          mode === action.mode ? "execute" : action.mode;
        if (nextMode === "research" && !desktop.capabilities.research) {
          setSlashCommandError("Research is unavailable for this target.");
          return "preserve";
        }
        if (nextMode !== "plan") {
          setPlanRevision(null);
        }
        setMode(nextMode);
        pushToast(
          `${nextMode === "plan" ? "Plan" : nextMode === "research" ? "Research" : "Execute"} mode enabled.`,
          "info",
        );
        return "clear";
      }
      case "show_mode_status":
        pushToast(
          action.mode === "plan"
            ? mode === "plan"
              ? planRevision === null
                ? "Plan mode is active. The next prompt creates a new durable draft."
                : `Plan mode is revising revision ${planRevision.revision}.`
              : "Plan mode is off."
            : mode === "research"
              ? "Research mode is active."
              : "Research mode is off.",
          "info",
        );
        return "clear";
      case "show_approval_mode":
        pushToast(
          `Desktop permission mode is ${desktop.approvalMode.replace("_", " ")}.`,
          "info",
        );
        return "clear";
      case "set_approval_mode": {
        const selected = desktop.targets.find(
          (target) => target.targetId === desktop.selectedTargetId,
        );
        if (selected?.kind !== "managed_local") {
          setSlashCommandError(
            "Desktop permission commands require a selected Managed Local target.",
          );
          return "preserve";
        }
        const changed = await handleSetApprovalMode(action.mode);
        if (!changed) {
          return "preserve";
        }
        pushToast(
          `Desktop permission mode changed to ${action.mode.replace("_", " ")}.`,
          "success",
        );
        return "clear";
      }
      case "select_surface":
        if (
          action.surface === "fleet" &&
          !desktop.capabilities.delegation &&
          !desktop.capabilities.agentWorkflows &&
          !desktop.capabilities.plugins
        ) {
          setSlashCommandError("Agents are unavailable for this target.");
          return "preserve";
        }
        if (action.surface === "library" && !desktop.capabilities.artifacts) {
          setSlashCommandError("Artifacts are unavailable for this target.");
          return "preserve";
        }
        setWorkNavigationOpen(false);
        setSurface(action.surface);
        return "clear";
      case "select_session_view":
        setWorkNavigationOpen(false);
        setSurface("work");
        setActiveSessionWorkspaceView(action.view);
        return "clear";
      case "open_tui": {
        const selected = desktop.targets.find(
          (target) => target.targetId === desktop.selectedTargetId,
        );
        if (
          !desktop.capabilities.tui ||
          !desktop.terminalEnabled ||
          selected?.terminalAvailable !== true
        ) {
          setSlashCommandError(
            "Enable the authenticated local TUI for a ready Managed Local target in Settings.",
          );
          return "preserve";
        }
        await handleOpenTerminal("colossus_tui");
        return "clear";
      }
    }
  }

  async function submitRun(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const cleanPrompt = prompt.trim();
    const slashCommand = parseDesktopSlashCommand(cleanPrompt);
    if (slashCommand.type === "invalid") {
      setSlashCommandError(slashCommand.message);
      return;
    }
    if (slashCommand.type === "action") {
      const disposition = await handleDesktopSlashAction(slashCommand.action);
      if (disposition === "clear") {
        setPrompt("");
      }
      return;
    }
    if (submitInFlight.current || connectingRef.current) {
      return;
    }
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
    setConversationFollowRequest((current) => current + 1);
    const continuationQueueLength =
      continuationView === undefined
        ? 0
        : messagesForThread(
            queuedMessagesRef.current,
            route.targetId,
            continuationView.run.sessionId,
          ).length;
    if (
      planRevision === null &&
      continuationView !== undefined &&
      (!isTerminalStatus(continuationView.run.status) ||
        continuationQueueLength > 0)
    ) {
      await enqueueCurrentMessage(continuationView, route, "last");
      return;
    }

    const sessionId = continuationView?.run.sessionId;
    const effectiveMode: RunMode = planRevision === null ? mode : "plan";
    if (effectiveMode === "research" && researchSources.length === 0) {
      setComposerError({
        ...FALLBACK_ACTION_ERROR,
        code: "invalid_argument",
        message: "Select at least one evidence source for Research.",
      });
      return;
    }
    const fingerprint = operationFingerprint([
      cleanPrompt,
      ...pluginSelections,
      route.targetId,
      sessionId ?? "",
      cleanRole,
      effectiveMode,
      ...(effectiveMode === "research"
        ? [researchDepth, ...researchSources]
        : []),
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
    createAttempt.current = { targetId: route.targetId, attempt };
    setComposerError(null);
    const result = await performRunSubmission(
      {
        prompt: cleanPrompt,
        pluginSkillIds: [...pluginSelections],
        attachments: [...attachments],
        role: cleanRole,
        mode: effectiveMode,
        researchDepth,
        researchSources,
        maxTurns,
        idempotencyKey: attempt.key,
        ...(sessionId === undefined ? {} : { sessionId }),
        ...(planRevision === null ? {} : { planRevision }),
      },
      route,
    );
    if (result.type === "accepted") {
      createAttempt.current = null;
      setPrompt("");
      setPlanRevision(null);
      setAttachments([]);
    } else if (result.type === "failed") {
      setComposerError(result.error);
    }
  }

  const deliverQueuedMessage = useCallback(
    async (message: QueuedMessage, route: TargetRoute) => {
      if (queueDeliveryRef.current !== null) {
        return;
      }
      queueDeliveryRef.current = message.id;
      commitQueuedMessages(
        updateQueuedMessage(
          queuedMessagesRef.current,
          message.id,
          (current) => ({ ...current, state: "sending", error: null }),
        ),
      );
      try {
        const result = await performRunSubmission(
          {
            prompt: message.prompt,
            ...(message.executionPrompt === undefined
              ? {}
              : { executionPrompt: message.executionPrompt }),
            pluginSkillIds: message.pluginSkillIds ?? [],
            attachments: message.attachments,
            role: message.role,
            mode: message.mode,
            researchDepth: message.researchDepth,
            researchSources: message.researchSources,
            maxTurns: message.maxTurns,
            idempotencyKey: message.idempotencyKey,
            sessionId: message.sessionId,
          },
          route,
        );
        if (result.type === "accepted") {
          commitQueuedMessages(
            removeQueuedMessage(queuedMessagesRef.current, message.id),
          );
          return;
        }
        if (result.type === "failed") {
          commitQueuedMessages(
            updateQueuedMessage(
              queuedMessagesRef.current,
              message.id,
              (current) => ({
                ...current,
                state: "failed",
                error: result.error,
              }),
            ),
          );
          return;
        }
        commitQueuedMessages(
          updateQueuedMessage(
            queuedMessagesRef.current,
            message.id,
            (current) => ({ ...current, state: "pending", error: null }),
          ),
        );
      } finally {
        queueDeliveryRef.current = null;
      }
    },
    [commitQueuedMessages, performRunSubmission],
  );

  async function editQueuedMessage(messageId: string, nextPrompt: string) {
    if (submitInFlight.current || connectingRef.current) return;
    if (!isPromptWithinByteLimit(nextPrompt)) {
      setComposerError({
        ...FALLBACK_ACTION_ERROR,
        code: "prompt_too_large",
        message: `Queued messages must be ${MAX_PROMPT_BYTES.toLocaleString()} UTF-8 bytes or fewer.`,
        retryable: false,
      });
      return;
    }
    const original = queuedMessagesRef.current.find(
      (message) => message.id === messageId,
    );
    const route = targetRoutes.current?.capture() ?? null;
    if (
      original === undefined ||
      route === null ||
      original.targetId !== route.targetId ||
      original.state === "sending" ||
      original.error?.outcomeUnknown === true
    )
      return;
    const sticky =
      original.conversationSkillIds ?? original.pluginSkillIds ?? [];
    let resolved = { prompt: nextPrompt.trim(), pluginSkillIds: [...sticky] };
    if (!FIXTURE_MODE && nextPrompt.trim().startsWith("@")) {
      submitInFlight.current = true;
      setSubmitting(true);
      try {
        resolved = await resolvePluginSelection(
          route.targetId,
          nextPrompt.trim(),
          sticky,
        );
        if (targetRoutes.current?.isCurrent(route) !== true) return;
      } catch (error: unknown) {
        setComposerError(commandError(error));
        return;
      } finally {
        submitInFlight.current = false;
        setSubmitting(false);
      }
    }
    commitQueuedMessages(
      updateQueuedMessage(queuedMessagesRef.current, messageId, (message) =>
        message.state === "sending" || message.error?.outcomeUnknown === true
          ? message
          : {
              ...message,
              prompt: nextPrompt.trim(),
              executionPrompt: resolved.prompt,
              pluginSkillIds: resolved.pluginSkillIds,
              idempotencyKey: crypto.randomUUID(),
              state: "pending",
              error: null,
            },
      ),
    );
    setComposerError(null);
  }

  function deleteQueuedMessage(messageId: string) {
    const message = queuedMessagesRef.current.find(
      (candidate) => candidate.id === messageId,
    );
    if (message?.state === "sending") {
      return;
    }
    commitQueuedMessages(
      removeQueuedMessage(queuedMessagesRef.current, messageId),
    );
  }

  function retryQueuedMessage(messageId: string) {
    commitQueuedMessages(
      updateQueuedMessage(queuedMessagesRef.current, messageId, (message) =>
        message.state === "failed"
          ? { ...message, state: "pending", error: null }
          : message,
      ),
    );
  }

  async function redirectCurrentResponse() {
    if (submitInFlight.current || connectingRef.current) {
      return;
    }
    const activeView =
      chatRef.current.activeRunId === null
        ? undefined
        : chatRef.current.views.get(chatRef.current.activeRunId);
    if (
      activeView === undefined ||
      isTerminalStatus(activeView.run.status) ||
      !isCancelable(activeView.run.status)
    ) {
      return;
    }
    const route =
      targetRoutes.current?.routeForRun(activeView.run.runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      setComposerError({
        ...FALLBACK_ACTION_ERROR,
        code: "disconnected",
        message: "The active response is no longer bound to this target.",
      });
      return;
    }
    const queued = await enqueueCurrentMessage(activeView, route, "next");
    if (queued === null) {
      return;
    }
    setConversationFollowRequest((current) => current + 1);
    if (!(await cancelActiveRun())) {
      setComposerError({
        ...FALLBACK_ACTION_ERROR,
        code: "redirect_not_started",
        message:
          "Your guidance is saved in Next up, but the current response could not be stopped. Retry Stop or let it finish.",
      });
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
      ...pluginSelections,
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
      pluginSkillIds: [...pluginSelections],
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
          archived: false,
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

  async function cancelActiveRun(): Promise<boolean> {
    if (connectingRef.current) {
      return false;
    }
    const activeView =
      chatRef.current.activeRunId === null
        ? undefined
        : chatRef.current.views.get(chatRef.current.activeRunId);
    if (activeView === undefined || !isCancelable(activeView.run.status)) {
      return false;
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
      return true;
    }

    const route = targetRoutes.current?.routeForRun(runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "disconnected",
        message: "The active run is no longer bound to this target.",
      });
      return false;
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
        return false;
      }
      dispatch({ type: "upsert_run", run });
      startWatch(runId, activeView.lastSequence, route);
      return true;
    } catch (error: unknown) {
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return false;
      }
      const failure = commandError(error);
      markConnectionFailure(failure, route);
      setActionError(failure);
      return false;
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

  async function loadAsides(parentSessionId: string) {
    const route = targetRoutes.current?.capture() ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      return;
    }
    setAsideError(null);
    if (FIXTURE_MODE) {
      return;
    }
    try {
      setAsideHistory(await listAsides(route.targetId, parentSessionId));
    } catch (error: unknown) {
      setAsideError(commandError(error));
    }
  }

  async function createAsideRun(
    promptText: string,
    draft: AsideDraft,
  ): Promise<boolean> {
    const route = targetRoutes.current?.capture() ?? null;
    const sourceView = chatRef.current.views.get(draft.sourceRunId);
    if (
      route === null ||
      sourceView === undefined ||
      targetRoutes.current?.isCurrent(route) !== true ||
      asideBusy
    ) {
      return false;
    }
    const visiblePrompt =
      draft.quote === ""
        ? promptText
        : `Regarding this excerpt:\n\n${draft.quote}\n\n${promptText}`;
    if (!isPromptWithinByteLimit(visiblePrompt)) {
      setAsideError({
        ...FALLBACK_ACTION_ERROR,
        code: "prompt_too_large",
        message: `Aside messages must be ${MAX_PROMPT_BYTES.toLocaleString()} UTF-8 bytes or fewer, including selected text.`,
        retryable: false,
      });
      return false;
    }
    const fingerprint = operationFingerprint([
      route.targetId,
      draft.sourceRunId,
      visiblePrompt,
    ]);
    const previous =
      asideCreateAttempt.current?.targetId === route.targetId
        ? asideCreateAttempt.current.attempt
        : null;
    const attempt = stableIdempotentAttempt(previous, fingerprint);
    asideCreateAttempt.current = { targetId: route.targetId, attempt };
    setAsideBusy(true);
    setAsideError(null);
    try {
      let run: Run;
      if (FIXTURE_MODE) {
        const now = new Date().toISOString();
        const identity = crypto.randomUUID();
        run = {
          ...sourceView.run,
          runId: `fixture-aside-${identity}`,
          sessionId: `fixture-aside-session-${identity}`,
          title: safeDisplayLabel(promptText, "Untitled Aside", 80),
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
                "This is a separate Aside response. The main thread remains unchanged.",
              profile: "desktop-showcase",
              modelProfile: "desktop-showcase",
              providerProfile: "fixture-provider",
              model: "fixture",
              elapsedSeconds: 0.1,
            },
          },
          etag: `fixture-etag-${identity}`,
          archived: false,
        };
      } else {
        run = await createRun(route.targetId, {
          prompt: visiblePrompt,
          role: sourceView.run.role,
          mode: "execute",
          maxTurns: USE_CONFIGURED_MAX_TURNS,
          idempotencyKey: attempt.key,
          branch: {
            sourceRunId: draft.sourceRunId,
          },
        });
      }
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return false;
      }
      targetRoutes.current.bindRun(run.runId, route);
      dispatchAside({ type: "upsert_run", run });
      dispatchAside({
        type: "record_local_prompt",
        runId: run.runId,
        prompt: visiblePrompt,
      });
      dispatchAside({ type: "select_run", runId: run.runId });
      setAsideReadOnly(false);
      asideCreateAttempt.current = null;
      startAsideWatch(run.runId, 0, route);
      return true;
    } catch (error: unknown) {
      setAsideError(commandError(error));
      return false;
    } finally {
      setAsideBusy(false);
    }
  }

  async function continueAsideRun(
    promptText: string,
    current: RunView,
  ): Promise<boolean> {
    const route = targetRoutes.current?.routeForRun(current.run.runId) ?? null;
    if (
      route === null ||
      targetRoutes.current?.isCurrent(route) !== true ||
      asideBusy ||
      !isTerminalStatus(current.run.status)
    ) {
      return false;
    }
    if (!isPromptWithinByteLimit(promptText)) {
      setAsideError({
        ...FALLBACK_ACTION_ERROR,
        code: "prompt_too_large",
        message: `Aside messages must be ${MAX_PROMPT_BYTES.toLocaleString()} UTF-8 bytes or fewer.`,
        retryable: false,
      });
      return false;
    }
    setAsideBusy(true);
    setAsideError(null);
    const fingerprint = operationFingerprint([
      route.targetId,
      current.run.sessionId,
      promptText,
    ]);
    const previous =
      asideCreateAttempt.current?.targetId === route.targetId
        ? asideCreateAttempt.current.attempt
        : null;
    const attempt = stableIdempotentAttempt(previous, fingerprint);
    asideCreateAttempt.current = { targetId: route.targetId, attempt };
    try {
      const run = FIXTURE_MODE
        ? {
            ...current.run,
            runId: `fixture-aside-${crypto.randomUUID()}`,
            title: safeDisplayLabel(promptText, "Untitled Aside", 80),
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
          }
        : await createRun(route.targetId, {
            prompt: promptText,
            sessionId: current.run.sessionId,
            role: current.run.role,
            mode: "execute",
            maxTurns: USE_CONFIGURED_MAX_TURNS,
            idempotencyKey: attempt.key,
          });
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return false;
      }
      targetRoutes.current.bindRun(run.runId, route);
      dispatchAside({ type: "upsert_run", run });
      dispatchAside({
        type: "record_local_prompt",
        runId: run.runId,
        prompt: promptText,
      });
      dispatchAside({ type: "select_run", runId: run.runId });
      setAsideReadOnly(false);
      asideCreateAttempt.current = null;
      startAsideWatch(run.runId, 0, route);
      return true;
    } catch (error: unknown) {
      setAsideError(commandError(error));
      return false;
    } finally {
      setAsideBusy(false);
    }
  }

  async function openAside(aside: Aside) {
    const route = targetRoutes.current?.capture() ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      return;
    }
    setAsideBusy(true);
    setAsideError(null);
    try {
      const details = FIXTURE_MODE
        ? { run: aside.run, pendingInteractions: [] }
        : await getRun(route.targetId, { runId: aside.run.runId });
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      targetRoutes.current.bindRun(details.run.runId, route);
      dispatchAside({ type: "reset" });
      dispatchAside({ type: "hydrate_run", details });
      dispatchAside({ type: "select_run", runId: details.run.runId });
      setAsideReadOnly(aside.closed || details.run.archived);
      if (!isTerminalStatus(details.run.status)) {
        startAsideWatch(details.run.runId, details.run.lastSequence, route);
      }
    } catch (error: unknown) {
      setAsideError(commandError(error));
    } finally {
      setAsideBusy(false);
    }
  }

  async function closeAside(current: RunView | undefined): Promise<boolean> {
    if (current === undefined) {
      dispatchAside({ type: "reset" });
      setAsideReadOnly(false);
      return true;
    }
    const route = targetRoutes.current?.routeForRun(current.run.runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      return false;
    }
    setAsideBusy(true);
    setAsideError(null);
    try {
      let run = current.run;
      if (!isTerminalStatus(run.status) && !FIXTURE_MODE) {
        run = await cancelRun(route.targetId, {
          runId: run.runId,
          idempotencyKey: crypto.randomUUID(),
        });
        for (
          let attempt = 0;
          attempt < 40 && !isTerminalStatus(run.status);
          attempt += 1
        ) {
          await new Promise((resolve) => window.setTimeout(resolve, 250));
          run = (await getRun(route.targetId, { runId: run.runId })).run;
        }
      }
      if (!isTerminalStatus(run.status) && !FIXTURE_MODE) {
        throw new Error(
          "The Aside is still stopping. Try closing it again shortly.",
        );
      }
      if (!FIXTURE_MODE) {
        await archiveThread(route.targetId, {
          runId: run.runId,
          idempotencyKey: crypto.randomUUID(),
        });
      }
      dispatchAside({ type: "reset" });
      setAsideReadOnly(false);
      if (activeRun !== undefined) {
        await loadAsides(activeRun.sessionId);
      }
      return true;
    } catch (error: unknown) {
      setAsideError(commandError(error));
      return false;
    } finally {
      setAsideBusy(false);
    }
  }

  async function respondAside(
    interaction: Interaction,
    response: InteractionAnswer,
  ) {
    if (FIXTURE_MODE) {
      dispatchAside({
        type: "interaction_resolved",
        interaction: { ...interaction, status: "answered" },
      });
      return;
    }
    const route = targetRoutes.current?.routeForRun(interaction.runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      return;
    }
    const resolved = await respondInteraction(route.targetId, {
      runId: interaction.runId,
      interactionId: interaction.interactionId,
      etag: interaction.etag,
      idempotencyKey: crypto.randomUUID(),
      response,
    });
    dispatchAside({ type: "interaction_resolved", interaction: resolved });
    const cursor = asideChat.views.get(interaction.runId)?.lastSequence ?? 0;
    startAsideWatch(interaction.runId, cursor, route);
  }

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
          executionBoundary: request.executionBoundary,
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
          executionBoundary: request.executionBoundary,
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
    setConversationSkills({});
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

  async function handleSelectSpace(spaceId: string) {
    if (
      spaceId === desktop.selectedSpaceId ||
      connectingRef.current ||
      submitInFlight.current
    ) {
      setWorkNavigationOpen(false);
      return;
    }
    const requestedSpace = desktopRef.current.spaces.find(
      (space) => space.spaceId === spaceId,
    );
    setSpaceStartup({
      spaceId,
      displayName: requestedSpace?.displayName ?? "Workspace",
    });
    connectingRef.current = true;
    if (!FIXTURE_MODE) {
      invalidateTargetRoute();
    }
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        if (requestedSpace === undefined) {
          return;
        }
        const status: DesktopStatus = {
          ...desktopRef.current,
          selectedSpaceId: spaceId,
          selectedTargetId: requestedSpace.targetId,
          connection: {
            ...desktopRef.current.connection,
            targetId: requestedSpace.targetId,
          },
          workspace: {
            workspaceId: requestedSpace.spaceId,
            displayName: requestedSpace.displayName,
            displayPath: requestedSpace.displayPath,
          },
          spaces: desktopRef.current.spaces.map((space) => ({
            ...space,
            selected: space.spaceId === spaceId,
          })),
        };
        desktopRef.current = status;
        setDesktop(status);
        setSurface("work");
        setWorkNavigationOpen(false);
        return;
      }
      const status = await selectSpace(spaceId);
      await acceptDesktopStatus(status, true);
      setShowOnboarding(managedOnboardingRequired(status));
      setSurface("work");
      setWorkNavigationOpen(false);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
      setSpaceStartup(null);
    }
  }

  async function handleCreateSpace() {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    setSpaceStartup({ spaceId: null, displayName: "New Workspace" });
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      if (FIXTURE_MODE) {
        return;
      }
      const status = await createSpace();
      if (status === null) {
        return;
      }
      invalidateTargetRoute();
      await acceptDesktopStatus(status, true);
      setShowOnboarding(managedOnboardingRequired(status));
      setSurface("work");
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
      setSpaceStartup(null);
    }
  }

  async function handleRenameSpace(spaceId: string, displayName: string) {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    try {
      if (!FIXTURE_MODE) {
        await acceptDesktopStatus(
          await renameSpace(spaceId, displayName),
          false,
        );
      }
    } catch (error: unknown) {
      setActionError(commandError(error));
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleArchiveSpace(spaceId: string) {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    const displayName =
      desktopRef.current.spaces.find((space) => space.spaceId === spaceId)
        ?.displayName ?? "Workspace";
    setSpaceActionFeedback({
      tone: "progress",
      message: `Archiving ${displayName}…`,
    });
    try {
      if (FIXTURE_MODE) {
        const wasSelected = desktopRef.current.selectedSpaceId === spaceId;
        const status = projectSpaceArchived(desktopRef.current, spaceId);
        desktopRef.current = status;
        setDesktop(status);
        if (wasSelected) {
          invalidateTargetRoute();
          dispatch({ type: "reset" });
        }
        setSpaceActionFeedback({
          tone: "success",
          message: `Archived ${displayName}.`,
        });
        return;
      }
      const wasSelected = desktopRef.current.selectedSpaceId === spaceId;
      if (wasSelected) {
        invalidateTargetRoute();
      }
      const status = await archiveSpace(spaceId);
      await acceptDesktopStatus(status, wasSelected);
      setShowOnboarding(managedOnboardingRequired(status));
      setSpaceActionFeedback({
        tone: "success",
        message: `Archived ${displayName}.`,
      });
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
      setSpaceActionFeedback({ tone: "error", message: failure.message });
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleRestoreSpace(spaceId: string) {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    const displayName =
      desktopRef.current.spaces.find((space) => space.spaceId === spaceId)
        ?.displayName ?? "Workspace";
    setSpaceActionFeedback({
      tone: "progress",
      message: `Restoring ${displayName}…`,
    });
    try {
      if (FIXTURE_MODE) {
        const status = projectSpaceRestored(desktopRef.current, spaceId);
        desktopRef.current = status;
        setDesktop(status);
      } else {
        await acceptDesktopStatus(await restoreSpace(spaceId), false);
      }
      setSpaceActionFeedback({
        tone: "success",
        message: `Restored ${displayName}. Select it when you’re ready.`,
      });
    } catch (error: unknown) {
      const failure = commandError(error);
      setActionError(failure);
      setSpaceActionFeedback({ tone: "error", message: failure.message });
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  }

  async function handleSelectSearchResult(result: SpaceSearchResult) {
    if (connectingRef.current || submitInFlight.current) {
      return;
    }
    if (FIXTURE_MODE) {
      if (desktopRef.current.selectedSpaceId !== result.spaceId) {
        await handleSelectSpace(result.spaceId);
      }
      const run = chatRef.current.recentRuns.find(
        (candidate) => candidate.runId === result.runId,
      );
      if (run !== undefined) {
        await openRun(run);
      }
      return;
    }

    const startsAnotherSpace =
      result.archived || desktopRef.current.selectedSpaceId !== result.spaceId;
    if (startsAnotherSpace) {
      setSpaceStartup({
        spaceId: result.spaceId,
        displayName: result.spaceName,
      });
    }
    connectingRef.current = true;
    setConnecting(true);
    setActionError(null);
    setRunLoadError("");
    try {
      let status = desktopRef.current;
      if (result.archived) {
        status = await restoreSpace(result.spaceId);
      }
      if (status.selectedSpaceId !== result.spaceId) {
        invalidateTargetRoute();
        status = await selectSpace(result.spaceId);
        await acceptDesktopStatus(status, true);
      } else if (result.archived) {
        await acceptDesktopStatus(status, false);
      }

      const route = targetRoutes.current?.capture() ?? null;
      if (
        route === null ||
        route.targetId !== result.targetId ||
        targetRoutes.current?.isCurrent(route) !== true
      ) {
        throw new CommandFailure({
          ...FALLBACK_ACTION_ERROR,
          code: "target_changed",
          message: "The Workspace changed before the thread could be opened.",
        });
      }
      const details = await getRun(route.targetId, { runId: result.runId });
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      targetRoutes.current.bindRun(details.run.runId, route);
      dispatch({ type: "hydrate_run", details });
      dispatch({ type: "select_run", runId: details.run.runId });
      setPlanRevision(null);
      setWorkQuery("");
      setSurface("work");
      setWorkNavigationOpen(false);
      startWatch(details.run.runId, details.run.lastSequence, route);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setRunLoadError(failure.message);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
      if (startsAnotherSpace) {
        setSpaceStartup(null);
      }
    }
  }

  async function handleArchiveThread(run: Run) {
    if (
      connectingRef.current ||
      submitInFlight.current ||
      threadLifecycleBusySessionId !== null
    ) {
      return;
    }
    if (!isTerminalStatus(run.status)) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "invalid_state",
        message: "Finish or cancel this thread before archiving it.",
      });
      return;
    }
    if (
      queuedMessagesRef.current.some(
        (message) => message.sessionId === run.sessionId,
      )
    ) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "thread_has_queued_messages",
        message:
          "Let Next up finish, or delete its queued messages, before archiving this thread.",
        retryable: false,
      });
      return;
    }
    const spaceId = desktopRef.current.selectedSpaceId;
    if (FIXTURE_MODE) {
      if (spaceId !== null) {
        updateThreadPin(spaceId, run.sessionId, false);
      }
      dispatch({ type: "remove_session", sessionId: run.sessionId });
      return;
    }
    const route = targetRoutes.current?.routeForRun(run.runId) ?? null;
    if (route === null || targetRoutes.current?.isCurrent(route) !== true) {
      setActionError({
        ...FALLBACK_ACTION_ERROR,
        code: "target_changed",
        message: "The thread is no longer bound to the selected Workspace.",
      });
      return;
    }
    const attemptKey = `${route.targetId}:${run.sessionId}:archive`;
    const attempt = stableIdempotentAttempt(
      threadLifecycleAttempts.current.get(attemptKey) ?? null,
      operationFingerprint([route.targetId, run.sessionId, "archive"]),
    );
    threadLifecycleAttempts.current = withBoundedEntry(
      threadLifecycleAttempts.current,
      attemptKey,
      attempt,
    );
    setThreadLifecycleBusySessionId(run.sessionId);
    setActionError(null);
    try {
      await archiveThread(route.targetId, {
        runId: run.runId,
        idempotencyKey: attempt.key,
      });
      threadLifecycleAttempts.current = withoutEntry(
        threadLifecycleAttempts.current,
        attemptKey,
      );
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      for (const recent of chatRef.current.recentRuns) {
        if (recent.sessionId === run.sessionId) {
          watchedRuns.current.delete(recent.runId);
        }
      }
      if (spaceId !== null) {
        updateThreadPin(spaceId, run.sessionId, false);
      }
      dispatch({ type: "remove_session", sessionId: run.sessionId });
      await loadRuns("", false, route);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure, route);
      setActionError(failure);
    } finally {
      setThreadLifecycleBusySessionId(null);
    }
  }

  async function handleRestoreThread(result: SpaceSearchResult) {
    if (
      connectingRef.current ||
      submitInFlight.current ||
      threadLifecycleBusySessionId !== null
    ) {
      return;
    }
    if (FIXTURE_MODE) {
      return;
    }
    const startsAnotherSpace =
      result.archived || desktopRef.current.selectedSpaceId !== result.spaceId;
    if (startsAnotherSpace) {
      setSpaceStartup({
        spaceId: result.spaceId,
        displayName: result.spaceName,
      });
    }
    connectingRef.current = true;
    setConnecting(true);
    setThreadLifecycleBusySessionId(result.sessionId);
    setActionError(null);
    try {
      let status = desktopRef.current;
      if (result.archived) {
        status = await restoreSpace(result.spaceId);
      }
      if (status.selectedSpaceId !== result.spaceId) {
        invalidateTargetRoute();
        status = await selectSpace(result.spaceId);
        await acceptDesktopStatus(status, true);
      } else if (result.archived) {
        await acceptDesktopStatus(status, false);
      }
      const route = targetRoutes.current?.capture() ?? null;
      if (
        route === null ||
        route.targetId !== result.targetId ||
        targetRoutes.current?.isCurrent(route) !== true
      ) {
        throw new CommandFailure({
          ...FALLBACK_ACTION_ERROR,
          code: "target_changed",
          message: "The Workspace changed before the thread could be restored.",
        });
      }
      const before = await getRun(route.targetId, { runId: result.runId });
      targetRoutes.current.bindRun(before.run.runId, route);
      const attemptKey = `${route.targetId}:${result.sessionId}:restore`;
      const attempt = stableIdempotentAttempt(
        threadLifecycleAttempts.current.get(attemptKey) ?? null,
        operationFingerprint([route.targetId, result.sessionId, "restore"]),
      );
      threadLifecycleAttempts.current = withBoundedEntry(
        threadLifecycleAttempts.current,
        attemptKey,
        attempt,
      );
      await restoreThread(route.targetId, {
        runId: result.runId,
        idempotencyKey: attempt.key,
      });
      threadLifecycleAttempts.current = withoutEntry(
        threadLifecycleAttempts.current,
        attemptKey,
      );
      if (targetRoutes.current?.isCurrent(route) !== true) {
        return;
      }
      const details = await getRun(route.targetId, { runId: result.runId });
      targetRoutes.current.bindRun(details.run.runId, route);
      dispatch({ type: "hydrate_run", details });
      dispatch({ type: "select_run", runId: details.run.runId });
      await loadRuns("", false, route);
      setWorkQuery("");
      setSurface("work");
      setWorkNavigationOpen(false);
    } catch (error: unknown) {
      const failure = commandError(error);
      markConnectionFailure(failure);
      setActionError(failure);
      await resyncDesktopAfterFailedMutation();
    } finally {
      connectingRef.current = false;
      setConnecting(false);
      setThreadLifecycleBusySessionId(null);
      if (startsAnotherSpace) {
        setSpaceStartup(null);
      }
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

  async function handleSetApprovalMode(
    approvalMode: ApprovalMode,
  ): Promise<boolean> {
    if (approvalModeChanging || submitting || connectingRef.current) {
      return false;
    }
    setApprovalModeChanging(true);
    setActionError(null);
    try {
      const status = FIXTURE_MODE
        ? { ...desktopRef.current, approvalMode }
        : await setApprovalMode(approvalMode);
      desktopRef.current = status;
      setDesktop(status);
      return true;
    } catch (error: unknown) {
      setActionError(commandError(error));
      await resyncDesktopAfterFailedMutation();
      return false;
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
  useEffect(() => {
    delegateRequest.current = null;
    dispatchDelegate({ type: "reset" });
    setSelectedDelegateId(null);
    setSelectedDelegateRunId(null);
    setDelegateLoading(false);
    setDelegateError("");
    setDelegateInspection(null);
  }, [activeRun?.sessionId, desktop.selectedSpaceId]);
  const activeRoute =
    activeRun === undefined
      ? null
      : (targetRoutes.current?.routeForRun(activeRun.runId) ?? null);
  const requestSessionMap = useCallback(
    async (runId: string, sessionId: string, showLoading: boolean) => {
      if (sessionMapRequest.current !== null) {
        return;
      }
      const request = Symbol(runId);
      sessionMapRequest.current = request;
      if (showLoading) {
        setSessionMapLoading(true);
      }
      setSessionMapError("");
      try {
        const next = await getSessionMap(runId);
        if (
          sessionMapRequest.current === request &&
          next.sessionId === sessionId
        ) {
          setSessionMap(next);
        }
      } catch (error: unknown) {
        if (sessionMapRequest.current === request) {
          if (showLoading) {
            setSessionMap(null);
          }
          setSessionMapError(commandError(error).message);
        }
      } finally {
        if (sessionMapRequest.current === request) {
          sessionMapRequest.current = null;
          setSessionMapLoading(false);
        }
      }
    },
    [],
  );
  useEffect(() => {
    const run = activeRun;
    if (run === undefined) {
      sessionMapRequest.current = null;
      setSessionMap(null);
      setSessionMapLoading(false);
      setSessionMapError("");
      return;
    }
    if (FIXTURE_MODE) {
      setSessionMap(developmentFixtures().buildSessionMapFixture());
      setSessionMapLoading(false);
      setSessionMapError("");
      return;
    }
    if (activeRoute?.kind !== "managed_local") {
      sessionMapRequest.current = null;
      setSessionMap(null);
      setSessionMapLoading(false);
      setSessionMapError(
        "Session resources are available for Managed Local Workspaces.",
      );
      return;
    }
    void requestSessionMap(run.runId, run.sessionId, true);
    return () => {
      sessionMapRequest.current = null;
    };
  }, [
    activeRoute?.generation,
    activeRoute?.kind,
    activeRun?.runId,
    activeRun?.sessionId,
    requestSessionMap,
  ]);
  useEffect(() => {
    const run = activeRun;
    if (
      !["topology", "snapshots", "resources"].includes(
        activeSessionWorkspaceView,
      ) ||
      run === undefined ||
      FIXTURE_MODE ||
      activeRoute?.kind !== "managed_local"
    ) {
      return undefined;
    }
    void requestSessionMap(run.runId, run.sessionId, false);
    const timer = window.setInterval(
      () => void requestSessionMap(run.runId, run.sessionId, false),
      3_000,
    );
    return () => window.clearInterval(timer);
  }, [
    activeRoute?.generation,
    activeRoute?.kind,
    activeRun?.runId,
    activeRun?.sessionId,
    activeSessionWorkspaceView,
    requestSessionMap,
  ]);
  const activeQueuedMessages = useMemo(
    () =>
      activeRun === undefined || activeRoute === null
        ? []
        : messagesForThread(
            queuedMessages,
            activeRoute.targetId,
            activeRun.sessionId,
          ),
    [activeRoute, activeRun, queuedMessages],
  );
  useEffect(() => {
    if (
      activeRun === undefined ||
      activeRoute === null ||
      !isTerminalStatus(activeRun.status) ||
      connecting ||
      submitting ||
      queueDeliveryRef.current !== null ||
      targetRoutes.current?.isCurrent(activeRoute) !== true
    ) {
      return;
    }
    const next = nextPendingMessage(
      queuedMessagesRef.current,
      activeRoute.targetId,
      activeRun.sessionId,
    );
    if (next !== undefined) {
      void deliverQueuedMessage(next, activeRoute);
    }
  }, [
    activeRoute,
    activeRun,
    connecting,
    deliverQueuedMessage,
    queuedMessages,
    submitting,
  ]);
  const conversationViews = useMemo(
    () => selectConversationViews(chat, activeRun?.sessionId ?? null),
    [activeRun?.sessionId, chat],
  );
  const asideView =
    asideChat.activeRunId === null
      ? undefined
      : asideChat.views.get(asideChat.activeRunId);
  const asideConversationViews = useMemo(
    () => selectConversationViews(asideChat, asideView?.run.sessionId ?? null),
    [asideChat, asideView?.run.sessionId],
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
    !approvalModeChanging;
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
    return selectSessionParticipants(conversationViews);
  }, [activeView, conversationViews]);
  const delegateView =
    selectedDelegateRunId === null
      ? undefined
      : delegateChat.views.get(selectedDelegateRunId);

  async function openDelegateParticipant(participant: AgentParticipant) {
    if (participant.kind !== "delegate" || activeRun === undefined) {
      return;
    }
    const parentRunId = participant.parentRunId ?? activeRun.runId;
    const parentView = chat.views.get(parentRunId);
    if (
      parentView === undefined ||
      parentView.run.sessionId !== activeRun.sessionId
    ) {
      setDelegateError(
        "This delegated run is not available in the selected session.",
      );
      setDelegateLoading(false);
      return;
    }
    const request = Symbol(`${parentRunId}:${participant.id}`);
    delegateRequest.current = request;
    dispatchDelegate({ type: "reset" });
    setSelectedDelegateId(participant.id);
    setSelectedDelegateRunId(null);
    setDelegateLoading(true);
    setDelegateError("");
    setDelegateInspection(null);

    if (FIXTURE_MODE) {
      const fixture = buildDelegateFixture(participant, parentView.run);
      dispatchDelegate({ type: "hydrate_run", details: fixture.details });
      for (const update of fixture.updates) {
        dispatchDelegate({ type: "ingest_update", update });
      }
      dispatchDelegate({
        type: "watch_complete",
        runId: fixture.details.run.runId,
      });
      setSelectedDelegateRunId(fixture.details.run.runId);
      setDelegateLoading(false);
      return;
    }

    const route = targetRoutes.current?.routeForRun(parentRunId) ?? null;
    if (
      route === null ||
      targetRoutes.current?.isCurrent(route) !== true ||
      (participant.parentRunId !== undefined &&
        participant.parentRunId !== parentRunId)
    ) {
      setDelegateError(
        "This delegated run is not available in the selected Workspace.",
      );
      setDelegateLoading(false);
      return;
    }

    try {
      const inspection = await getThreadDelegate(parentRunId, participant.id);
      if (
        delegateRequest.current !== request ||
        targetRoutes.current?.isCurrent(route) !== true ||
        inspection.jobId !== participant.id ||
        inspection.parentRunId !== parentRunId
      ) {
        return;
      }
      setDelegateInspection(inspection);
    } catch (error: unknown) {
      if (delegateRequest.current === request) {
        setDelegateError(commandError(error).message);
      }
    } finally {
      if (delegateRequest.current === request) {
        setDelegateLoading(false);
      }
    }
  }

  function closeDelegateInspector() {
    delegateRequest.current = null;
    setSelectedDelegateId(null);
    setSelectedDelegateRunId(null);
    setDelegateLoading(false);
    setDelegateError("");
    setDelegateInspection(null);
  }
  const selectedTarget = desktop.targets.find(
    (target) => target.targetId === desktop.selectedTargetId,
  );
  const terminalAvailable = selectedTarget?.terminalAvailable === true;
  const workspaceFilesAvailable = desktop.capabilities.files;
  const generatedTitle =
    openingRun?.title ?? conversationViews[0]?.run.title ?? activeRun?.title;
  const title =
    activeRun === undefined || generatedTitle === undefined
      ? "New work"
      : safeDisplayLabel(
          resolveThreadTitle(
            desktop.selectedSpaceId,
            activeRun.sessionId,
            generatedTitle,
          ),
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
      pluginSkills={completionSkills}
      pluginSelections={pluginSelections}
      onRemovePluginSkill={(id) =>
        setConversationSkills((current) => ({
          ...current,
          [selectionKey]: (current[selectionKey] ?? []).filter(
            (selected) => selected !== id,
          ),
        }))
      }
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
      researchDepth={researchDepth}
      researchSources={researchSources}
      researchAvailable={desktop.capabilities.research === true}
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
      queueing={
        (activeRun !== undefined && !isTerminalStatus(activeRun.status)) ||
        activeQueuedMessages.length > 0
      }
      activeWorkRunning={
        activeRun !== undefined && !isTerminalStatus(activeRun.status)
      }
      activeWorkNeedsInput={(activeView?.pendingInteractions.length ?? 0) > 0}
      activeWorkRedirectable={
        activeRun !== undefined && isCancelable(activeRun.status) && !cancelling
      }
      queuedMessages={activeQueuedMessages}
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
      onResearchDepthChange={setResearchDepth}
      onResearchSourcesChange={setResearchSources}
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
      onEditQueuedMessage={editQueuedMessage}
      onDeleteQueuedMessage={deleteQueuedMessage}
      onRetryQueuedMessage={retryQueuedMessage}
      onRedirect={() => void redirectCurrentResponse()}
      onSubmit={(event) => void submitRun(event)}
    />
  );
  const onboardingRequired = managedOnboardingRequired(desktop);
  const onboardingActive = showOnboarding || onboardingRequired;
  const developerPreview = releaseChannel === "developer_preview";
  const unsafeExecutionBannerVisible = executionBoundaryBannerVisible(
    desktop.managedState,
    desktop.executionBoundary,
  );

  return (
    <div
      ref={appShellRef}
      className={`app-shell${developerPreview ? " app-shell--developer-preview" : ""}${unsafeExecutionBannerVisible ? " app-shell--unsafe-execution" : ""}`}
      style={
        workSidebarWidthRef.current === null
          ? undefined
          : ({
              "--work-sidebar-width": `${workSidebarWidthRef.current}px`,
            } as CSSProperties)
      }
    >
      <a className="skip-link" href="#primary-workspace">
        Skip to workspace
      </a>
      <ReleaseChannelBanner
        releaseChannel={releaseChannel}
        releaseMetadata={releaseMetadata}
      />
      <ExecutionBoundaryBanner
        active={managedRuntimeBoundaryActive(desktop.managedState)}
        boundary={desktop.executionBoundary}
      />
      <ToastRegion toasts={toasts} onDismiss={dismissToast} />
      {!onboardingActive && workNavigationOpen ? (
        <button
          className="workspace-drawer-backdrop work-navigation-backdrop"
          type="button"
          aria-label="Close work navigation"
          aria-hidden="true"
          tabIndex={-1}
          onClick={closeWorkNavigation}
        />
      ) : null}

      {onboardingActive ? null : (
        <WorkSidebar
          runs={chat.recentRuns}
          spaces={desktop.spaces}
          selectedSpaceId={desktop.selectedSpaceId}
          surface={surface}
          connectionState={connection.state}
          capabilities={desktop.capabilities}
          terminalEnabled={desktop.terminalEnabled}
          terminalAvailable={terminalAvailable}
          activeSessionId={activeRun?.sessionId ?? null}
          pinnedSessionIds={pinnedThreadSessionIds}
          resolveThreadTitle={resolveThreadTitle}
          query={workQuery}
          searchScope={searchScope}
          includeArchived={includeArchivedSearch}
          searchResults={spaceSearchResults}
          searchBusy={spaceSearchBusy}
          searchError={spaceSearchError}
          searchHasMore={spaceSearchCursor !== ""}
          spaceThreadPreviews={spaceThreadPreviews}
          spaceThreadPreviewBusyIds={spaceThreadPreviewBusyIds}
          spaceThreadPreviewErrors={spaceThreadPreviewErrors}
          busy={listBusy}
          error={listError}
          spaceActionFeedback={spaceActionFeedback}
          hasMore={chat.nextPageToken !== ""}
          disabled={submitting || (connecting && spaceStartup === null)}
          spaceStartup={spaceStartup}
          threadLifecycleBusySessionId={threadLifecycleBusySessionId}
          sidebarWidth={initialWorkSidebarWidth}
          drawerOpen={workNavigationOpen}
          onQueryChange={setWorkQuery}
          onSearchScopeChange={setSearchScope}
          onIncludeArchivedChange={setIncludeArchivedSearch}
          onNewWork={newWork}
          onSelect={(run) => void openRun(run)}
          onSelectSearchResult={(result) =>
            void handleSelectSearchResult(result)
          }
          onLoadMore={() => void loadRuns(chat.nextPageToken, true)}
          onLoadMoreSearch={() => void loadMoreSpaceSearch()}
          onLoadSpaceThreadPreview={(spaceId) =>
            void loadSpaceThreadPreview(spaceId)
          }
          onSelectSpace={(spaceId) => void handleSelectSpace(spaceId)}
          onCreateSpace={() => void handleCreateSpace()}
          onRenameSpace={(spaceId, displayName) =>
            void handleRenameSpace(spaceId, displayName)
          }
          onArchiveSpace={(spaceId) => void handleArchiveSpace(spaceId)}
          onRestoreSpace={(spaceId) => void handleRestoreSpace(spaceId)}
          onArchiveThread={(run) => void handleArchiveThread(run)}
          onRenameThread={handleRenameThread}
          onToggleThreadPinned={handleToggleThreadPinned}
          onRestoreThread={(result) => void handleRestoreThread(result)}
          onSelectSurface={selectSurface}
          onOpenTerminal={() => void handleOpenTerminal("colossus_tui")}
          onOpenShell={() => void handleOpenTerminal("shell")}
          onSidebarWidthPreview={previewWorkSidebarWidth}
          onSidebarWidthCommit={commitWorkSidebarWidth}
          onSidebarWidthReset={resetWorkSidebarWidth}
          onDrawerOpen={openWorkNavigation}
          onDrawerClose={closeWorkNavigation}
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
          sessionMap={sessionMap}
          sessionMapLoading={sessionMapLoading}
          sessionMapError={sessionMapError}
          selectedParticipantId={selectedDelegateId}
          delegateView={delegateView}
          delegateInspection={delegateInspection}
          delegateLoading={delegateLoading}
          delegateError={delegateError}
          artifacts={artifactItems}
          selectedSpaceName={
            desktop.spaces.find(
              (space) => space.spaceId === desktop.selectedSpaceId,
            )?.displayName ?? "Current Workspace"
          }
          threadPinned={
            activeRun !== undefined &&
            pinnedThreadSessionIds.has(activeRun.sessionId)
          }
          followRequestSequence={conversationFollowRequest}
          composer={composer}
          filesAvailable={desktop.capabilities.files}
          onOpenWorkspaceFile={(path) => {
            if (desktop.workspace === null) {
              return;
            }
            workspaceFileOpenSequence.current += 1;
            setWorkspaceFileOpenRequest({
              workspaceId: desktop.workspace.workspaceId,
              path,
              requestId: workspaceFileOpenSequence.current,
            });
          }}
          artifactsAvailable={desktop.capabilities.artifacts}
          asideView={asideView}
          asideConversationViews={asideConversationViews}
          asideHistory={asideHistory}
          asideBusy={asideBusy}
          asideError={asideError}
          asideReadOnly={asideReadOnly}
          planContinuationAvailable={desktop.capabilities.planContinuation}
          initialSessionWorkspaceView={activeSessionWorkspaceView}
          onSessionWorkspaceViewChange={setActiveSessionWorkspaceView}
          sessionActivityAvailable={
            desktop.capabilities.sessionActivity === true
          }
          loadSessionActivity={(request) => {
            if (FIXTURE_MODE) {
              let liveActivityCount = 0;
              if (FIXTURE_ACTIVITY_LIVE && request.pageToken === "") {
                const now = Date.now();
                fixtureActivityStartedAt.current ??= now;
                const elapsed = now - fixtureActivityStartedAt.current;
                liveActivityCount =
                  elapsed >= 5_000 ? 2 : elapsed >= 2_000 ? 1 : 0;
              }
              return Promise.resolve(
                developmentFixtures().buildSessionActivityFixture(request, {
                  liveActivityCount,
                }),
              );
            }
            const targetId = desktop.selectedTargetId;
            if (targetId === null) {
              return Promise.reject(
                new Error(
                  "Select a connected Colossus target to inspect activity.",
                ),
              );
            }
            return listSessionActivity(targetId, request);
          }}
          activityComparisonEnabled={FIXTURE_SCENARIO === "activity-comparison"}
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
              openRequest={workspaceFileOpenRequest}
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
          onSelectParticipant={(participant) =>
            void openDelegateParticipant(participant)
          }
          onBackToThreadDetails={closeDelegateInspector}
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
          onLoadAsides={loadAsides}
          onCreateAside={createAsideRun}
          onContinueAside={continueAsideRun}
          onOpenAside={openAside}
          onNewAside={() => {
            dispatchAside({ type: "reset" });
            setAsideError(null);
            setAsideReadOnly(false);
          }}
          onRespondAside={respondAside}
          onCloseAside={closeAside}
        />
      ) : (
        <OperationsSurface
          pluginSelections={pluginSelections}
          onUsePluginSkill={(id) => {
            setConversationSkills((current) => ({
              ...current,
              [selectionKey]: [
                ...new Set([...(current[selectionKey] ?? []), id]),
              ],
            }));
            setSurface("work");
            requestAnimationFrame(() => composerRef.current?.focus());
          }}
          surface={surface}
          connection={connection}
          desktop={desktop}
          connecting={connecting}
          updateChecking={updateChecking}
          updateMessage={updateMessage}
          runs={chat.recentRuns}
          artifacts={allArtifacts}
          demoParticipants={FIXTURE_MODE ? DEMO_PARTICIPANTS : null}
          workNavigationOpen={workNavigationOpen}
          onOpenWorkNavigation={openWorkNavigation}
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
