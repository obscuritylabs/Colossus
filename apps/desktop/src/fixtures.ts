import { MAX_FEED_ITEMS, MAX_RECENT_RUNS } from "./state";
import type { ChatState, RunView } from "./state";
import type {
  ArtifactReference,
  Interaction,
  Run,
  RunUpdate,
  TokenUsage,
} from "./types";

const SELECTED_RUN_ID = "fixture-run-desktop-release";
const SESSION_ID = "fixture-session-operations-studio";

function update(
  sequence: number,
  createdAt: string,
  kind: RunUpdate["update"],
): RunUpdate {
  return {
    runId: SELECTED_RUN_ID,
    sequence,
    createdAt,
    update: kind,
  };
}

function recentRun(
  runId: string,
  sessionId: string,
  role: string,
  status: Run["status"],
  createdAt: string,
  updatedAt: string,
  lastSequence: number,
): Run {
  const completed = status === "completed";
  return {
    runId,
    sessionId,
    role,
    mode: "execute",
    status,
    createdAt,
    updatedAt,
    startedAt: createdAt,
    finishedAt: completed ? updatedAt : null,
    lastSequence,
    pendingInteractionCount: 0,
    terminal: completed
      ? {
          type: "result",
          result: {
            output: "",
            profile: "desktop",
            modelProfile: "desktop",
            providerProfile: "fixture-provider",
            model: "openrouter/auto",
            elapsedSeconds: 74,
          },
        }
      : null,
    etag: `fixture-etag-${runId}`,
    selectedSkills: [],
  };
}

/**
 * Builds the deterministic Operations Studio state used while developing the
 * desktop UI. This function has no side effects and refuses to run in a
 * production build; callers should additionally guard it with
 * `import.meta.env.DEV` so production never imports fixture state by default.
 */
export function buildOperationsStudioFixture(): ChatState {
  if (!import.meta.env.DEV) {
    throw new Error(
      "Operations Studio fixtures are available in development only.",
    );
  }

  const releaseChecklist: ArtifactReference = {
    artifactId: "fixture-artifact-release-checklist",
    fileName: "bootstrap.rs",
    mediaType: "text/x-rust",
    sizeBytes: 18_432,
    sha256: "127b8b6f65f443d09a2fc3b60c8437cfaf4712ab2c914c6ce8ad5d1a9f06ed72",
    purpose: "run_input",
    state: "available",
    createdAt: "2026-07-20T14:30:00Z",
  };
  const readinessReport: ArtifactReference = {
    artifactId: "fixture-artifact-readiness-report",
    fileName: "bootstrap.spec.rs",
    mediaType: "text/x-rust",
    sizeBytes: 27_648,
    sha256: "733e7f6f585a64e501bc9445f25f9ab256071bf837a75e76c0c88e9009ad4331",
    purpose: "run_output",
    state: "available",
    createdAt: "2026-07-20T14:34:40Z",
  };
  const designNotes: ArtifactReference = {
    artifactId: "fixture-artifact-design-notes",
    fileName: "design-notes.md",
    mediaType: "text/markdown",
    sizeBytes: 7_168,
    sha256: "47233b431747147355977309c4a6f2c7f75a2b86cd613814cfb24d130ead9558",
    purpose: "run_output",
    state: "available",
    createdAt: "2026-07-20T14:34:40Z",
  };
  const approval: Interaction = {
    interactionId: "fixture-interaction-publish-report",
    runId: SELECTED_RUN_ID,
    kind: "approval",
    status: "pending",
    createdAt: "2026-07-20T14:35:00Z",
    expiresAt: "2026-07-20T15:05:00Z",
    respondableByCaller: true,
    etag: "fixture-etag-publish-report",
    content: {
      type: "approval",
      reason:
        "The bootstrap hardening is ready, but applying it changes the desktop runtime boundary.",
      action: "Apply the hardened bootstrap changes",
      resource: "workspace://colossus/apps/desktop/bootstrap.rs",
      risk: "medium",
      requestHash:
        "0c9fa2e338d64d60336a9ef5d655cf4785bad3eb530995f894ffaac40626d32b",
    },
  };
  const usage: TokenUsage = {
    inputTokens: 12_840,
    outputTokens: 2_416,
    totalTokens: 15_256,
    cachedInputTokens: 4_096,
    reasoningTokens: 1_184,
  };

  const updates: RunUpdate[] = [
    update(1, "2026-07-20T14:30:00Z", {
      type: "message",
      message: {
        sessionId: SESSION_ID,
        runId: SELECTED_RUN_ID,
        sequence: 1,
        role: "user",
        content: [
          {
            type: "text",
            text: "Harden the desktop agent bootstrap, coordinate implementation and security review, and leave the native runtime boundary unchanged without approval.",
          },
        ],
        createdAt: "2026-07-20T14:30:00Z",
      },
    }),
    update(2, "2026-07-20T14:30:08Z", {
      type: "reasoning_summary",
      summary:
        "Plan\n1. Trace the desktop bootstrap and connection lifecycle.\n2. Make startup failures explicit and recoverable.\n3. Ask the security agent to verify IPC and credential boundaries.\n4. Add focused tests before requesting approval.",
    }),
    update(3, "2026-07-20T14:30:14Z", {
      type: "tool_activity",
      activity: {
        callId: "fixture-call-inspect-workspace",
        toolName: "workspace.inspect",
        state: "started",
        summary:
          "Inspecting the desktop package, SDK boundary, and bootstrap lifecycle.",
      },
    }),
    update(4, "2026-07-20T14:31:22Z", {
      type: "tool_activity",
      activity: {
        callId: "fixture-call-inspect-workspace",
        toolName: "workspace.inspect",
        state: "completed",
        summary:
          "Reviewed 18 relevant files and isolated the recoverable bootstrap states.",
      },
    }),
    update(5, "2026-07-20T14:32:05Z", {
      type: "notice",
      reason: "agent_handoff_complete",
      message:
        "Sentinel completed a read-only security pass and confirmed the credential boundary remains native-only.",
    }),
    update(6, "2026-07-20T14:34:40Z", {
      type: "message",
      message: {
        sessionId: SESSION_ID,
        runId: SELECTED_RUN_ID,
        sequence: 6,
        role: "tool",
        content: [
          {
            type: "text",
            text: "Builder prepared the bootstrap patch and focused regression tests.",
          },
          { type: "artifact", artifact: designNotes },
          { type: "artifact", artifact: readinessReport },
          { type: "artifact", artifact: releaseChecklist },
        ],
        createdAt: "2026-07-20T14:34:40Z",
      },
    }),
    update(7, "2026-07-20T14:34:58Z", {
      type: "tool_activity",
      activity: {
        callId: "fixture-call-publish-report",
        toolName: "workspace.apply_patch",
        state: "waiting_approval",
        summary:
          "Waiting for approval to apply the reviewed bootstrap changes.",
      },
    }),
    update(8, "2026-07-20T14:35:00Z", {
      type: "interaction",
      interaction: approval,
    }),
    update(9, "2026-07-20T14:35:01Z", {
      type: "usage",
      usage,
    }),
  ];

  const selectedRun: Run = {
    runId: SELECTED_RUN_ID,
    sessionId: SESSION_ID,
    role: "harden-desktop-agent-bootstrap",
    mode: "execute",
    status: "waiting",
    createdAt: "2026-07-20T14:30:00Z",
    updatedAt: "2026-07-20T14:35:01Z",
    startedAt: "2026-07-20T14:30:02Z",
    finishedAt: null,
    lastSequence: 9,
    pendingInteractionCount: 1,
    terminal: null,
    etag: "fixture-etag-desktop-release",
    selectedSkills: ["release-management", "security-review", "documents"],
  };
  const selectedView: RunView = {
    run: selectedRun,
    localPrompt:
      "Harden the desktop agent bootstrap and coordinate implementation, security review, and tests.",
    output: `## Bootstrap ready

The bootstrap path is now **explicit**, recoverable, and covered by focused tests.

- [x] Credentials remain outside the webview
- [x] Privileged IPC stays behind narrow native commands
- [x] Renderer output is treated as untrusted content

| Review | Result |
| --- | --- |
| Security | Passed |
| Focused tests | Passed |

The reviewed patch is ready and waiting for your approval.`,
    updates: updates.slice(-MAX_FEED_ITEMS),
    seenSequences: new Set(updates.map(({ sequence }) => sequence)),
    lastSequence: selectedRun.lastSequence,
    pendingInteractions: [approval],
    usage,
    streamState: "watching",
    streamError: null,
  };

  const recentRuns = [
    selectedRun,
    recentRun(
      "fixture-run-sdk-contracts",
      "fixture-session-sdk-contracts",
      "draft-public-sdk-contracts",
      "running",
      "2026-07-20T14:28:00Z",
      "2026-07-20T14:34:12Z",
      14,
    ),
    recentRun(
      "fixture-run-security-baseline",
      "fixture-session-security-baseline",
      "audit-ipc-boundary",
      "completed",
      "2026-07-20T14:15:00Z",
      "2026-07-20T14:27:19Z",
      22,
    ),
    recentRun(
      "fixture-run-docs-polish",
      "fixture-session-docs-polish",
      "polish-operator-documentation",
      "completed",
      "2026-07-20T13:52:00Z",
      "2026-07-20T14:12:44Z",
      17,
    ),
    recentRun(
      "fixture-run-sso-desktop",
      "fixture-session-sso-desktop",
      "add-sso-to-desktop-app",
      "completed",
      "2026-07-20T13:30:00Z",
      "2026-07-20T13:48:12Z",
      18,
    ),
    recentRun(
      "fixture-run-file-sync",
      "fixture-session-file-sync",
      "fix-file-sync-on-reconnect",
      "completed",
      "2026-07-20T13:10:00Z",
      "2026-07-20T13:26:44Z",
      21,
    ),
    recentRun(
      "fixture-run-agent-protocol",
      "fixture-session-agent-protocol",
      "document-agent-protocol",
      "completed",
      "2026-07-20T12:42:00Z",
      "2026-07-20T13:04:17Z",
      19,
    ),
    recentRun(
      "fixture-run-diagnostics",
      "fixture-session-diagnostics",
      "improve-logs-and-diagnostics",
      "completed",
      "2026-07-20T12:10:00Z",
      "2026-07-20T12:37:22Z",
      24,
    ),
    recentRun(
      "fixture-run-grpc-v2",
      "fixture-session-grpc-v2",
      "migrate-to-grpc-v2",
      "completed",
      "2026-07-20T11:31:00Z",
      "2026-07-20T12:02:09Z",
      31,
    ),
    recentRun(
      "fixture-run-onboarding",
      "fixture-session-onboarding",
      "onboard-new-contractors",
      "completed",
      "2026-07-20T10:48:00Z",
      "2026-07-20T11:24:55Z",
      28,
    ),
  ].slice(0, MAX_RECENT_RUNS);

  return {
    activeRunId: SELECTED_RUN_ID,
    views: new Map([[SELECTED_RUN_ID, selectedView]]),
    recentRuns,
    nextPageToken: "",
  };
}
