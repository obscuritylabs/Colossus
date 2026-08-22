import { MAX_FEED_ITEMS, MAX_RECENT_RUNS } from "../state";
import type { ChatState, RunView } from "../state";
import type {
  ArtifactReference,
  Interaction,
  ListSessionActivityRequest,
  Run,
  RunUpdate,
  SessionActivity,
  SessionActivityPage,
  SessionMap,
  TokenUsage,
} from "../types";

export function buildSessionMapFixture(): SessionMap {
  return {
    sessionId: SESSION_ID,
    delegates: [
      {
        jobId: "fixture-delegate-builder",
        parentRunId: SELECTED_RUN_ID,
        childSessionId: "fixture-session-builder",
        childRunId: "fixture-run-builder",
        task: "Harden the desktop runtime boundary and verify its recovery path.",
        role: "implementation",
        status: "completed",
        finalOutput: "Implemented the bounded bootstrap recovery changes.",
        error: "",
        createdAt: "2026-07-20T14:30:30Z",
        updatedAt: "2026-07-20T14:33:10Z",
        startedAt: "2026-07-20T14:30:31Z",
        completedAt: "2026-07-20T14:33:10Z",
      },
      {
        jobId: "fixture-delegate-sentinel",
        parentRunId: SELECTED_RUN_ID,
        childSessionId: "fixture-session-sentinel",
        childRunId: "fixture-run-sentinel",
        task: "Review the Desktop IPC and credential boundaries.",
        role: "security",
        status: "completed",
        finalOutput: "No cross-Space control path was found.",
        error: "",
        createdAt: "2026-07-20T14:31:00Z",
        updatedAt: "2026-07-20T14:34:00Z",
        startedAt: "2026-07-20T14:31:01Z",
        completedAt: "2026-07-20T14:34:00Z",
      },
      {
        jobId: "fixture-delegate-scribe",
        parentRunId: SELECTED_RUN_ID,
        childSessionId: "fixture-session-scribe",
        task: "Prepare the operator-facing change summary.",
        role: "writer",
        status: "running",
        finalOutput: "",
        error: "",
        createdAt: "2026-07-20T14:33:00Z",
        updatedAt: "2026-07-20T14:35:00Z",
        startedAt: "2026-07-20T14:33:01Z",
      },
    ],
    goals: [
      {
        id: "fixture-goal-architecture",
        objective: "Review workspace architecture",
        sourcePlanId: "fixture-plan-bootstrap",
        status: "active",
        summary: "Two of five bounded iterations are complete.",
        blockedReason: "",
        iterationBudget: 5,
        iterationsCompleted: 2,
        createdAt: "2026-07-20T14:30:00Z",
        updatedAt: "2026-07-20T14:34:00Z",
      },
      {
        id: "fixture-goal-runtime",
        objective: "Harden desktop runtime",
        status: "complete",
        summary:
          "The bootstrap path now fails safely and reports recovery guidance.",
        blockedReason: "",
        iterationBudget: 5,
        iterationsCompleted: 5,
        createdAt: "2026-07-20T14:29:00Z",
        updatedAt: "2026-07-20T14:34:40Z",
      },
    ],
    tasks: [
      {
        id: "fixture-task-bootstrap",
        title: "Harden bootstrap recovery",
        description: "Make startup failures explicit and recoverable.",
        status: "completed",
        createdAt: "2026-07-20T14:30:00Z",
        updatedAt: "2026-07-20T14:34:00Z",
      },
      {
        id: "fixture-task-tests",
        title: "Verify selected-Space isolation",
        description: "Add negative IPC and renderer contract coverage.",
        status: "in_progress",
        createdAt: "2026-07-20T14:30:10Z",
        updatedAt: "2026-07-20T14:35:00Z",
      },
      {
        id: "fixture-task-docs",
        title: "Update recovery guidance",
        description: "Document safe restart and operator recovery steps.",
        status: "pending",
        createdAt: "2026-07-20T14:30:20Z",
        updatedAt: "2026-07-20T14:30:20Z",
      },
      {
        id: "fixture-task-release",
        title: "Run the Desktop completion gate",
        description: "Verify renderer, native, and Rust completion checks.",
        status: "pending",
        createdAt: "2026-07-20T14:30:30Z",
        updatedAt: "2026-07-20T14:30:30Z",
      },
    ],
    plans: [
      {
        id: "fixture-plan-bootstrap",
        prompt: "Harden desktop agent bootstrap",
        status: "draft",
        revision: 2,
        content:
          "## Bootstrap hardening\n\n1. Trace lifecycle boundaries.\n2. Make startup failures recoverable.\n3. Verify native isolation.\n4. Add focused tests.",
        stepCount: 4,
        createdAt: "2026-07-20T14:30:00Z",
        updatedAt: "2026-07-20T14:34:00Z",
      },
    ],
    decisions: [
      {
        id: "fixture-decision-boundary",
        planId: "fixture-plan-bootstrap",
        source: "user",
        status: "active",
        priority: "critical",
        title: "Keep execution boundary unchanged",
        decision:
          "Do not broaden the native execution boundary without explicit approval.",
        intent:
          "Preserve the current security posture while improving reliability.",
        appliesWhen: "Changing Desktop runtime or worker startup behavior.",
        rationale:
          "The reliability work does not require additional authority.",
        createdAt: "2026-07-20T14:30:00Z",
        updatedAt: "2026-07-20T14:30:00Z",
      },
      {
        id: "fixture-decision-fail-closed",
        goalId: "fixture-goal-runtime",
        source: "agent",
        status: "active",
        priority: "high",
        title: "Fail closed on identity drift",
        decision: "Stop the affected Space when workspace identity changes.",
        intent: "Avoid silently operating on a different folder object.",
        appliesWhen:
          "The selected Space no longer matches its canonical identity.",
        rationale: "An explicit reselection is safer than path-only recovery.",
        createdAt: "2026-07-20T14:31:00Z",
        updatedAt: "2026-07-20T14:31:00Z",
      },
    ],
    memories: [
      {
        id: "fixture-memory-rust",
        scope: "repository",
        kind: "rule",
        confidence: 1,
        source: "user",
        status: "active",
        text: "Use Rust 1.96 and edition 2024 for implementation work.",
        rationale: "Repository engineering requirement.",
        createdAt: "2026-07-20T14:30:00Z",
        updatedAt: "2026-07-20T14:32:00Z",
      },
      {
        id: "fixture-memory-tests",
        scope: "repository",
        kind: "preference",
        confidence: 0.95,
        source: "agent",
        status: "active",
        text: "Prefer focused tests while iterating.",
        rationale:
          "Fast feedback preserves momentum without replacing completion gates.",
        createdAt: "2026-07-20T14:31:00Z",
        updatedAt: "2026-07-20T14:31:00Z",
      },
      {
        id: "fixture-memory-research",
        scope: "session",
        kind: "preference",
        confidence: 0.9,
        source: "user",
        status: "active",
        text: "Use source-backed research for product decisions.",
        rationale: "This session compares interaction patterns and evidence.",
        createdAt: "2026-07-20T14:32:00Z",
        updatedAt: "2026-07-20T14:32:00Z",
      },
    ],
    researchRuns: [
      {
        id: "fixture-research-patterns",
        question:
          "Which Desktop session-map patterns preserve clarity at scale?",
        depth: "standard",
        sourceKinds: ["repo", "web"],
        status: "completed",
        queryCount: 3,
        sourceCount: 5,
        limitationCount: 1,
        report:
          "The strongest pattern combines progressive disclosure with a persistent inspector.",
        error: "",
        createdAt: "2026-07-20T14:30:00Z",
        updatedAt: "2026-07-20T14:34:00Z",
        completedAt: "2026-07-20T14:34:00Z",
      },
    ],
    researchSources: [
      {
        id: "fixture-source-architecture",
        runId: "fixture-research-patterns",
        label: "R1",
        kind: "repo",
        title: "Colossus Desktop architecture",
        uri: "workspace://docs/develop/architecture.md",
        query: "desktop architecture boundaries",
        createdAt: "2026-07-20T14:31:00Z",
      },
      {
        id: "fixture-source-security",
        runId: "fixture-research-patterns",
        label: "R2",
        kind: "repo",
        title: "Security architecture",
        uri: "workspace://docs/develop/security-architecture.md",
        query: "renderer native boundary",
        createdAt: "2026-07-20T14:31:10Z",
      },
    ],
  };
}

const SELECTED_RUN_ID = "fixture-run-desktop-release";
const SESSION_ID = "fixture-session-operations-studio";

function activity(
  sequence: number,
  turn: number,
  lane: SessionActivity["lane"],
  kind: SessionActivity["kind"],
  title: string,
  summary: string,
  startedAt: string,
  options: Partial<SessionActivity> = {},
): SessionActivity {
  return {
    activityId: `fixture-activity-${sequence}`,
    runId: SELECTED_RUN_ID,
    turn,
    lane,
    kind,
    title,
    summary,
    actor:
      kind === "user" ? "User" : lane === "system" ? "System" : "Assistant",
    status: "completed",
    startedAt,
    completedAt: null,
    durationMs: null,
    input: null,
    result: null,
    attributes: {},
    sourceEventTypes: [`fixture.${kind}.v1`],
    firstSequence: sequence,
    lastSequence: sequence,
    ...options,
  };
}

const SESSION_ACTIVITY_FIXTURE: SessionActivity[] = [
  activity(
    1,
    1,
    "agent",
    "user",
    "User message",
    "Review the desktop release path and identify its trust boundaries.",
    "2026-07-20T14:30:00.103Z",
    {
      input: {
        format: "text",
        value:
          "Review the desktop release path and identify its trust boundaries.",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    2,
    1,
    "system",
    "system",
    "Context prepared",
    "Prepared 6 messages from session state",
    "2026-07-20T14:30:00.145Z",
    {
      attributes: {
        message_count: "6",
        token_estimate: "1840",
        strategy: "full",
      },
      sourceEventTypes: ["context.prepared.v1"],
    },
  ),
  activity(
    3,
    1,
    "agent",
    "assistant",
    "Assistant turn",
    "Model request prepared",
    "2026-07-20T14:30:00.184Z",
    {
      completedAt: "2026-07-20T14:30:00.360Z",
      durationMs: 176,
      attributes: { model_profile: "primary", tool_count: "12" },
      sourceEventTypes: ["model.request.prepared.v1", "final.output.v1"],
    },
  ),
  activity(
    4,
    1,
    "tools",
    "tool",
    "filesystem.search",
    "Searched workspace files matching README",
    "2026-07-20T14:30:00.362Z",
    {
      completedAt: "2026-07-20T14:30:00.446Z",
      durationMs: 84,
      input: {
        format: "json",
        value: '{\n  "query": "**/README*",\n  "max_results": 50\n}',
      },
      result: {
        format: "text",
        value:
          "Found 3 released results\n./README.md\n./docs/README.md\n./examples/README.md",
      },
      attributes: { tool_name: "filesystem.search" },
      sourceEventTypes: [
        "tool.call.requested.v1",
        "tool.call.started.v1",
        "tool.call.completed.v1",
      ],
      lastSequence: 6,
    },
  ),
  activity(
    7,
    1,
    "tools",
    "tool",
    "filesystem.read",
    "Read the released architecture guide",
    "2026-07-20T14:30:00.448Z",
    {
      completedAt: "2026-07-20T14:30:00.550Z",
      durationMs: 102,
      input: {
        format: "json",
        value: '{ "path": "docs/develop/architecture.md" }',
      },
      result: {
        format: "text",
        value: "# Architecture\n\nRust is the active root implementation…",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    8,
    1,
    "system",
    "system",
    "Token usage",
    "2,184 tokens used",
    "2026-07-20T14:30:00.573Z",
    {
      attributes: {
        input_tokens: "1712",
        output_tokens: "472",
        total_tokens: "2184",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    9,
    2,
    "agent",
    "user",
    "User message",
    "Trace the native bootstrap and credential boundaries.",
    "2026-07-20T14:31:02.101Z",
    {
      input: {
        format: "text",
        value: "Trace the native bootstrap and credential boundaries.",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    10,
    2,
    "system",
    "system",
    "Context prepared",
    "Prepared 12 messages from session state",
    "2026-07-20T14:31:02.142Z",
    {
      attributes: {
        message_count: "12",
        token_estimate: "4630",
        strategy: "full",
      },
      sourceEventTypes: ["context.prepared.v1"],
    },
  ),
  activity(
    11,
    2,
    "tools",
    "tool",
    "shell.exec",
    "Checked the workspace status",
    "2026-07-20T14:31:02.360Z",
    {
      completedAt: "2026-07-20T14:31:02.451Z",
      durationMs: 91,
      input: {
        format: "json",
        value: '{ "command": "git status --porcelain" }',
      },
      result: {
        format: "text",
        value: "Process exited successfully with no released output.",
      },
      attributes: { exit_code: "0" },
      sourceEventTypes: ["tool.call.started.v1", "tool.call.completed.v1"],
      lastSequence: 12,
    },
  ),
  activity(
    13,
    2,
    "tools",
    "tool",
    "network.fetch",
    "Outbound request denied by policy",
    "2026-07-20T14:31:02.510Z",
    {
      status: "failed",
      completedAt: "2026-07-20T14:31:02.519Z",
      durationMs: 9,
      result: {
        format: "text",
        value: "The requested destination was not released by policy.",
      },
      attributes: { policy: "denied" },
      sourceEventTypes: ["tool.call.requested.v1", "effect.denied.v1"],
      lastSequence: 14,
    },
  ),
  activity(
    15,
    2,
    "agent",
    "assistant",
    "Reasoning summary",
    "The native sidecar binds the selected workspace identity before runtime composition.",
    "2026-07-20T14:31:02.689Z",
    {
      result: {
        format: "text",
        value:
          "The native sidecar binds the selected workspace identity before runtime composition.",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    16,
    3,
    "agent",
    "user",
    "User message",
    "Turn the findings into an implementation plan.",
    "2026-07-20T14:32:10.021Z",
    {
      input: {
        format: "text",
        value: "Turn the findings into an implementation plan.",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    17,
    3,
    "system",
    "system",
    "Context prepared",
    "Prepared 19 messages from session state",
    "2026-07-20T14:32:10.063Z",
    {
      attributes: {
        compacted: "true",
        message_count: "19",
        strategy: "summary",
      },
      sourceEventTypes: ["context.prepared.v1"],
    },
  ),
  activity(
    18,
    3,
    "system",
    "system",
    "Plan written",
    "A canonical Plan revision was saved",
    "2026-07-20T14:32:11.220Z",
    {
      attributes: { plan_id: "fixture-plan-bootstrap", revision: "3" },
      sourceEventTypes: ["plan.written.v1"],
    },
  ),
  activity(
    19,
    3,
    "agent",
    "assistant",
    "Assistant response",
    "Outlined the release boundary, verification gates, and recovery behavior.",
    "2026-07-20T14:32:11.240Z",
    {
      completedAt: "2026-07-20T14:32:11.583Z",
      durationMs: 343,
      result: {
        format: "text",
        value:
          "## Implementation plan\n\n1. Preserve the native trust boundary.\n2. Add bounded verification gates.\n3. Exercise recovery before release.",
      },
      sourceEventTypes: ["model.request.prepared.v1", "api.run.update.v1"],
      lastSequence: 20,
    },
  ),
  activity(
    21,
    4,
    "agent",
    "user",
    "User message",
    "Run the focused checks and summarize the release readiness.",
    "2026-07-20T14:33:12.103Z",
    {
      input: {
        format: "text",
        value: "Run the focused checks and summarize the release readiness.",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    22,
    4,
    "system",
    "system",
    "Context prepared",
    "Prepared 24 messages from session state",
    "2026-07-20T14:33:12.145Z",
    {
      attributes: {
        message_count: "24",
        token_estimate: "7890",
        strategy: "full",
      },
      sourceEventTypes: ["context.prepared.v1"],
    },
  ),
  activity(
    23,
    4,
    "agent",
    "assistant",
    "Assistant turn",
    "Planning the release verification sequence",
    "2026-07-20T14:33:12.184Z",
    {
      completedAt: "2026-07-20T14:33:12.360Z",
      durationMs: 176,
      attributes: { model_profile: "primary", tool_count: "12" },
      sourceEventTypes: ["model.request.prepared.v1", "final.output.v1"],
    },
  ),
  activity(
    24,
    4,
    "tools",
    "tool",
    "filesystem.search",
    "Located release and validation manifests",
    "2026-07-20T14:33:12.362Z",
    {
      completedAt: "2026-07-20T14:33:12.446Z",
      durationMs: 84,
      input: {
        format: "json",
        value: '{ "query": "**/{Cargo.toml,tauri.conf.json}" }',
      },
      result: {
        format: "text",
        value:
          "Found released manifests\n./Cargo.toml\n./apps/desktop/src-tauri/tauri.conf.json",
      },
      attributes: { tool_name: "filesystem.search" },
      sourceEventTypes: [
        "tool.call.requested.v1",
        "tool.call.started.v1",
        "tool.call.completed.v1",
      ],
      lastSequence: 26,
    },
  ),
  activity(
    27,
    4,
    "tools",
    "tool",
    "filesystem.read",
    "Read the released validation configuration",
    "2026-07-20T14:33:12.448Z",
    {
      completedAt: "2026-07-20T14:33:12.550Z",
      durationMs: 102,
      input: {
        format: "json",
        value: '{ "path": "apps/desktop/src-tauri/tauri.conf.json" }',
      },
      result: {
        format: "text",
        value:
          "Released configuration preview: local-only CSP and native capabilities.",
      },
      sourceEventTypes: ["tool.call.started.v1", "tool.call.completed.v1"],
      lastSequence: 29,
    },
  ),
  activity(
    30,
    4,
    "tools",
    "tool",
    "shell.exec",
    "Checked the candidate change set",
    "2026-07-20T14:33:12.552Z",
    {
      completedAt: "2026-07-20T14:33:12.643Z",
      durationMs: 91,
      input: {
        format: "json",
        value: '{ "command": "git status --short" }',
      },
      result: {
        format: "text",
        value:
          "Process exited successfully; released summary contains 12 changed paths.",
      },
      attributes: { exit_code: "0" },
      sourceEventTypes: ["tool.call.started.v1", "tool.call.completed.v1"],
      lastSequence: 32,
    },
  ),
  activity(
    33,
    4,
    "system",
    "system",
    "State checkpoint saved",
    "Saved the released turn checkpoint",
    "2026-07-20T14:33:12.646Z",
    { sourceEventTypes: ["api.run.update.v1"] },
  ),
  activity(
    34,
    4,
    "agent",
    "assistant",
    "Reasoning summary",
    "Focused checks cover the projection, transport boundary, and desktop release surface.",
    "2026-07-20T14:33:12.689Z",
    {
      result: {
        format: "text",
        value:
          "Focused checks cover the projection, transport boundary, and desktop release surface.",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    35,
    4,
    "system",
    "system",
    "Token usage",
    "8,412 tokens used",
    "2026-07-20T14:33:12.845Z",
    {
      attributes: {
        input_tokens: "7310",
        output_tokens: "1102",
        total_tokens: "8412",
      },
      sourceEventTypes: ["api.run.update.v1"],
    },
  ),
  activity(
    36,
    4,
    "tools",
    "tool",
    "cargo.test",
    "Ran focused Rust library tests",
    "2026-07-20T14:33:13.052Z",
    {
      completedAt: "2026-07-20T14:33:14.332Z",
      durationMs: 1280,
      input: {
        format: "json",
        value: '{ "package": "colossus-runtime", "lib": true }',
      },
      result: {
        format: "text",
        value: "test result: ok. 126 passed; 0 failed",
      },
      attributes: { exit_code: "0" },
      sourceEventTypes: [
        "tool.call.requested.v1",
        "tool.call.started.v1",
        "tool.call.completed.v1",
      ],
      lastSequence: 38,
    },
  ),
  activity(
    39,
    4,
    "agent",
    "assistant",
    "Assistant response",
    "The release boundary is ready for the full validation gate.",
    "2026-07-20T14:33:14.389Z",
    {
      completedAt: "2026-07-20T14:33:14.732Z",
      durationMs: 343,
      result: {
        format: "text",
        value:
          "Focused checks pass. The native boundary remains fail-closed and is ready for the full release gate.",
      },
      sourceEventTypes: ["model.request.prepared.v1", "api.run.update.v1"],
      lastSequence: 40,
    },
  ),
  activity(
    41,
    4,
    "system",
    "system",
    "Run completed",
    "Run completed",
    "2026-07-20T14:33:14.745Z",
    { sourceEventTypes: ["run.completed.v1"] },
  ),
].sort((left, right) => right.firstSequence - left.firstSequence);

export function buildSessionActivityFixture(
  request: ListSessionActivityRequest,
): SessionActivityPage {
  const query = request.query?.trim().toLocaleLowerCase() ?? "";
  const lanes = request.lanes ?? [];
  const kinds = request.kinds ?? [];
  const statuses = request.statuses ?? [];
  const matching = SESSION_ACTIVITY_FIXTURE.filter((item) => {
    if (lanes.length > 0 && !lanes.includes(item.lane)) return false;
    if (kinds.length > 0 && !kinds.includes(item.kind)) return false;
    if (
      statuses.length > 0 &&
      (item.status === null || !statuses.includes(item.status))
    ) {
      return false;
    }
    return (
      query === "" ||
      JSON.stringify({
        title: item.title,
        summary: item.summary,
        actor: item.actor,
        attributes: item.attributes,
        sourceEventTypes: item.sourceEventTypes,
        input: item.input?.value,
        result: item.result?.value,
      })
        .toLocaleLowerCase()
        .includes(query)
    );
  });
  const offset =
    Number.parseInt(request.pageToken?.replace("fixture:", "") ?? "0", 10) || 0;
  const pageSize = Math.min(Math.max(request.pageSize ?? 100, 1), 100);
  const page = matching.slice(offset, offset + pageSize);
  const nextOffset = offset + page.length;
  return {
    activities: page,
    nextPageToken: nextOffset < matching.length ? `fixture:${nextOffset}` : "",
    headSequence: 41,
    projectedThroughSequence: 41,
    caughtUp: true,
  };
}

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
    title: `${role.charAt(0).toUpperCase()}${role.slice(1).replaceAll("-", " ")}`,
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
    archived: false,
  };
}

/**
 * Builds the deterministic Operations Studio state used while developing the
 * desktop UI. This function has no side effects and refuses to run in a
 * production build; callers should additionally guard it with
 * `import.meta.env.DEV` so production never imports fixture state by default.
 */
export function buildOperationsStudioFixture(
  interactionKind: Interaction["kind"] = "approval",
): ChatState {
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
  const userPrompt: Interaction = {
    interactionId: "fixture-interaction-language-question",
    runId: SELECTED_RUN_ID,
    kind: "user_prompt",
    status: "pending",
    createdAt: "2026-07-20T14:35:00Z",
    expiresAt: "2026-07-20T15:05:00Z",
    respondableByCaller: true,
    etag: "fixture-etag-language-question",
    content: {
      type: "user_prompt",
      question: "What's your favorite programming language?",
      choices: [
        { choiceId: "javascript", label: "JavaScript" },
        { choiceId: "python", label: "Python" },
        { choiceId: "go", label: "Go" },
        { choiceId: "rust", label: "Rust" },
      ],
      allowFreeForm: false,
    },
  };
  const pendingInteraction =
    interactionKind === "user_prompt" ? userPrompt : approval;
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
      interaction: pendingInteraction,
    }),
    update(9, "2026-07-20T14:35:01Z", {
      type: "usage",
      usage,
    }),
  ];

  const selectedRun: Run = {
    runId: SELECTED_RUN_ID,
    sessionId: SESSION_ID,
    title: "Harden desktop agent bootstrap",
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
    archived: false,
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
    pendingInteractions: [pendingInteraction],
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
      "waiting",
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

/**
 * Builds one completed turn with released reasoning summaries between tool
 * calls. The renderer uses this development-only fixture to compare the two
 * activity presentations against identical canonical data.
 */
export function buildActivityComparisonFixture(): ChatState {
  const state = buildOperationsStudioFixture();
  const current = state.views.get(SELECTED_RUN_ID);
  if (current === undefined) {
    throw new Error("The activity comparison fixture requires a selected run.");
  }

  const usage: TokenUsage = {
    inputTokens: 3_981,
    outputTokens: 684,
    totalTokens: 4_665,
    cachedInputTokens: 1_024,
    reasoningTokens: 312,
  };
  const updates: RunUpdate[] = [
    update(1, "2026-08-15T14:42:00Z", {
      type: "message",
      message: {
        sessionId: SESSION_ID,
        runId: SELECTED_RUN_ID,
        sequence: 1,
        role: "user",
        content: [
          {
            type: "text",
            text: "Review this workspace and identify the safest high-impact next task",
          },
        ],
        createdAt: "2026-08-15T14:42:00Z",
      },
    }),
    update(2, "2026-08-15T14:42:01Z", {
      type: "reasoning_summary",
      summary: "Orienting to the workspace",
    }),
    update(3, "2026-08-15T14:42:01Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-map",
        toolName: "repo.map_structure",
        state: "requested",
        summary: "Validated repository mapping request",
        input: '{"depth":3,"include_hidden":false}',
      },
    }),
    update(4, "2026-08-15T14:42:02Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-map",
        toolName: "repo.map_structure",
        state: "completed",
        summary: "Mapped repository structure",
        preview:
          '{"root":".","top_level":["proposal","compliance","submission"],"file_count":42}',
      },
    }),
    update(5, "2026-08-15T14:42:03Z", {
      type: "reasoning_summary",
      summary: "The proposal package is nearly complete",
    }),
    update(6, "2026-08-15T14:42:04Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-read",
        toolName: "repo.read_many",
        state: "requested",
        summary: "Requested six proposal files",
        input:
          '{"paths":["README.md","proposal.md","compliance.md","submission/checklist.md","review/notes.md","CHANGELOG.md"]}',
      },
    }),
    update(7, "2026-08-15T14:42:05Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-read",
        toolName: "repo.read_many",
        state: "completed",
        summary: "Inspected 6 proposal files",
        preview: '{"files_read":6,"required_sections":12,"missing_sections":0}',
      },
    }),
    update(8, "2026-08-15T14:42:06Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-readiness",
        toolName: "shell.run",
        state: "requested",
        summary: "Validated submission readiness command",
        input: '{"command":"git status --short","cwd":"."}',
      },
    }),
    update(9, "2026-08-15T14:42:07Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-readiness",
        toolName: "shell.run",
        state: "completed",
        summary: "Checked submission readiness",
        preview: "Working tree clean",
      },
    }),
    update(10, "2026-08-15T14:42:07Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-policy",
        toolName: "shell.run",
        state: "requested",
        summary: "Validated workspace policy command",
        input: '{"command":"gh pr checks --required","cwd":".","network":true}',
      },
    }),
    update(11, "2026-08-15T14:42:08Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-policy",
        toolName: "shell.run",
        state: "failed",
        summary: "Command denied by workspace policy",
      },
    }),
    update(12, "2026-08-15T14:42:09Z", {
      type: "reasoning_summary",
      summary: "Policy blocks shell access; will pivot to read-only analysis",
    }),
    update(13, "2026-08-15T14:42:09Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-status",
        toolName: "repo.search",
        state: "requested",
        summary: "Requested released activity and CI summaries",
        input: '{"query":"submission OR ci status","limit":20}',
      },
    }),
    update(14, "2026-08-15T14:42:11Z", {
      type: "tool_activity",
      activity: {
        callId: "comparison-status",
        toolName: "repo.search",
        state: "completed",
        summary: "Reviewed recent activity and CI status",
        preview:
          '{"submission_ready":true,"ci_status":"blocked","blocked_by":"ci:full label"}',
      },
    }),
    update(15, "2026-08-15T14:42:12Z", { type: "usage", usage }),
  ];

  const run: Run = {
    ...current.run,
    title: "Review workspace readiness",
    role: "primary",
    status: "completed",
    createdAt: "2026-08-15T14:42:00Z",
    updatedAt: "2026-08-15T14:42:12Z",
    startedAt: "2026-08-15T14:42:00Z",
    finishedAt: "2026-08-15T14:42:12Z",
    lastSequence: 15,
    pendingInteractionCount: 0,
    terminal: {
      type: "result",
      result: {
        output: "Review complete.",
        profile: "desktop",
        modelProfile: "desktop",
        providerProfile: "fixture-provider",
        model: "fixture",
        elapsedSeconds: 12,
      },
    },
  };
  const view: RunView = {
    ...current,
    run,
    localPrompt: null,
    output: `The workspace is well-prepared for submission, but one required CI check is blocked by repository policy.

**Safest high-impact next task:** ask a repository writer to apply the required \`ci:full\` label, then validate the proposal package end-to-end without elevating shell access.`,
    updates,
    seenSequences: new Set(updates.map(({ sequence }) => sequence)),
    lastSequence: 15,
    pendingInteractions: [],
    usage,
    streamState: "complete",
    streamError: null,
  };

  return {
    ...state,
    activeRunId: run.runId,
    views: new Map([[run.runId, view]]),
    recentRuns: [
      run,
      ...state.recentRuns.filter((candidate) => candidate.runId !== run.runId),
    ],
  };
}

/** Builds a completed Plan Mode turn for the in-chat lifecycle fixture. */
export function buildPlanWorkflowFixture(): ChatState {
  if (!import.meta.env.DEV) {
    throw new Error(
      "Plan workflow fixtures are available in development only.",
    );
  }
  const runId = "fixture-run-plan-workflow";
  const sessionId = "fixture-session-plan-workflow";
  const createdAt = "2026-07-30T18:20:00Z";
  const run: Run = {
    runId,
    sessionId,
    title: "Plan the Desktop release workflow",
    role: "primary",
    mode: "plan",
    status: "completed",
    createdAt,
    updatedAt: "2026-07-30T18:20:18Z",
    startedAt: createdAt,
    finishedAt: "2026-07-30T18:20:18Z",
    lastSequence: 2,
    pendingInteractionCount: 0,
    terminal: {
      type: "result",
      result: {
        output: "Plan saved.",
        planId: "plan-fixture-desktop-release",
        planRevision: 3,
        planStatus: "draft",
        profile: "desktop",
        modelProfile: "desktop",
        providerProfile: "fixture-provider",
        model: "fixture",
        elapsedSeconds: 18,
      },
    },
    etag: "fixture-etag-plan-workflow",
    selectedSkills: [],
    archived: false,
  };
  const updates: RunUpdate[] = [
    {
      runId,
      sequence: 1,
      createdAt,
      update: {
        type: "message",
        message: {
          sessionId,
          runId,
          sequence: 1,
          role: "user",
          content: [
            {
              type: "text",
              text: "Create a safe release plan for the Desktop Plan workflow.",
            },
          ],
          createdAt,
        },
      },
    },
    {
      runId,
      sequence: 2,
      createdAt: "2026-07-30T18:20:18Z",
      update: {
        type: "notice",
        reason: "plan.written",
        message:
          "plan plan-fixture-desktop-release was persisted at revision 3",
      },
    },
  ];
  const view: RunView = {
    run,
    localPrompt: null,
    output: `## Desktop Plan workflow

1. Add an exact revision-bound public Plan continuation.
2. Keep revision in structurally constrained Plan Mode.
3. Route direct and Goal execution through durable public runs.
4. Preserve policy prompts, cancellation, audit, and the advanced TUI workflow.
5. Verify the chat decision card at compact and full widths.`,
    updates,
    seenSequences: new Set([1, 2]),
    lastSequence: 2,
    pendingInteractions: [],
    usage: {
      inputTokens: 2_840,
      outputTokens: 612,
      totalTokens: 3_452,
      cachedInputTokens: 1_024,
      reasoningTokens: 184,
    },
    streamState: "complete",
    streamError: null,
  };
  return {
    activeRunId: runId,
    views: new Map([[runId, view]]),
    recentRuns: [run],
    nextPageToken: "",
  };
}
