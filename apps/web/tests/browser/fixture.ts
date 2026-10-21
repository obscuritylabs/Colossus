import { expect, type Page } from "@playwright/test";
import type { FleetNode, Thread, ThreadDetailResponse } from "../../src/api";
import type { ChatSettingsSnapshot } from "../../src/chat-settings";
import type { Me } from "../../src/control-api";
import type { RunRequest } from "../../src/RunComposer";
import type { ProviderPreset } from "@colossus/ui/lib/provider-catalog";
import type { CredentialSnapshot } from "../../src/CredentialStoreSettings";

const timestamp = "2026-10-16T10:00:00Z";
const base = "/api/projects/browser-project";
const capabilities = [
  "runtime.resources.v1",
  "runtime.model_catalog.v1",
  "agent_runs.model_selection",
  "research.create",
  "goal.create",
  "plans.continue",
];
const node: FleetNode = {
  node: {
    project_id: "browser-project",
    node_id: "browser-node",
    instance_id: "browser-instance",
    label: "Browser test workspace",
    certificate_sha256: "synthetic-certificate",
    roles: ["primary"],
    revoked: false,
    revision: 1,
    policy: {
      schema_version: 1,
      provenance: "runtime",
      fingerprint: "synthetic-policy",
      configuration_revision: 1,
      access_profile: "browser-fixture",
      sandbox_backend: "browser-fixture",
      sandbox_profile: "browser-fixture",
      boundary_acknowledged: true,
      approval_mode: "always",
      allowed_roles: ["primary"],
      allowed_tools: ["filesystem.search"],
      capabilities: [],
      models: [],
      findings: [],
      telemetry: {
        provenance: "runtime",
        denied_requests: null,
        approval_requests: null,
        outcome_unknown_runs: null,
      },
    },
  },
  presence: {
    ready: true,
    connection_id: "browser-connection",
    capabilities,
  },
};
const me: Me = {
  user: {
    id: "browser-user",
    display_name: "Browser Test Operator",
    email: null,
    active: true,
    is_admin: false,
    revision: 1,
    created_at: timestamp,
    updated_at: timestamp,
  },
  memberships: [
    {
      project_id: "browser-project",
      user_id: "browser-user",
      subject: "browser-user",
      role: "operator",
      permissions: ["read", "execute", "control"],
    },
  ],
  projects: [
    {
      id: "browser-project",
      name: "Browser test project",
      description: "Synthetic browser acceptance data",
      parent_project_id: null,
      archived: false,
      revision: 1,
      created_at: timestamp,
      updated_at: timestamp,
    },
  ],
};
interface Submission {
  path: string;
  node_id?: string;
  expected_revision?: number;
  request: RunRequest;
}

/** Run the production App against a closed synthetic API, never a controller. */
export async function browserFixture(
  page: Page,
  options: {
    administer?: boolean;
    globalAdmin?: boolean;
    running?: boolean;
    plan?: boolean;
    projectName?: string;
    hostLabel?: string;
    managedChat?: boolean;
    permissions?: Me["memberships"][number]["permissions"];
    settingsOverride?: Partial<ChatSettingsSnapshot>;
    providerPresets?: ProviderPreset[];
  } = {},
) {
  const identity = {
    ...me,
    user: { ...me.user, is_admin: Boolean(options.globalAdmin) },
    memberships: me.memberships.map((membership) => ({
      ...membership,
      permissions: options.permissions ?? membership.permissions,
    })),
    projects: me.projects.map((project) => ({
      ...project,
      name: options.projectName ?? project.name,
    })),
  };
  const host = options.hostLabel
    ? {
        host_id: "browser-host",
        project_id: "browser-project",
        label: options.hostLabel,
        platform: "macos",
        deployment_kind: "cli",
        revision: 1,
        last_seen_at: 1792461600,
      }
    : null;
  const runtimeNode = {
    ...node,
    node: {
      ...node.node,
      node_id: options.managedChat ? `chat-${"a".repeat(64)}` : "browser-node",
      ...(host
        ? {
            host_id: host.host_id,
            workspace_label: "CLI workspace",
          }
        : {}),
    },
  };
  const nodeId = runtimeNode.node.node_id;
  const submissions: Submission[] = [],
    unexpected: string[] = [],
    errors: string[] = [];
  const thread: Thread = {
    thread_id: "browser-thread",
    project_id: "browser-project",
    node_id: nodeId,
    session_id: "browser-session",
    title: "Browser model conversation",
    revision: 1,
    created_at: timestamp,
    updated_at: timestamp,
    archived: false,
    source: "cloud",
    sync_status: "current",
    can_continue: true,
    ...(options.plan || options.managedChat
      ? { owner_user_id: me.user.id }
      : {}),
    ...(host ? { host_id: host.host_id } : {}),
  };
  const messages: ThreadDetailResponse["messages"] = options.managedChat
    ? [
        {
          message_id: "retained-personal-question",
          role: "user",
          text: "Retained personal chat question",
          created_at: timestamp,
        },
      ]
    : [];
  let created = Boolean(options.plan || options.managedChat),
    chatStarted = Boolean(options.plan || options.managedChat),
    personalConnected = false;
  const tasks: ThreadDetailResponse["tasks"] = options.plan
    ? [
        {
          task_id: "browser-plan-task",
          project_id: "browser-project",
          node_id: nodeId,
          subject: me.user.id,
          request: {
            input: [{ text: "Draft a controlled plan" }],
            mode: "plan",
            role: "primary",
          },
          run_id: "browser-plan-run",
          revision: 1,
          last_sequence: 0,
          snapshot: {
            run: {
              run_id: "browser-plan-run",
              session_id: "browser-session",
              title: "Controlled browser plan",
              status: "completed",
              role: "primary",
              mode: "plan",
              created_at: timestamp,
              updated_at: timestamp,
              last_sequence: 0,
              terminal: {
                result: {
                  output:
                    "## Proposed steps\n\n1. Review connected sources.\n2. Summarize the evidence.",
                  plan_id: "browser-plan",
                  plan_revision: 2,
                  plan_status: "draft",
                },
              },
            },
            pending_interactions: [],
          },
        },
      ]
    : [];
  if (options.running) {
    thread.active_task_id = "browser-active-task";
    tasks.push({
      task_id: "browser-active-task",
      project_id: "browser-project",
      node_id: nodeId,
      subject: me.user.id,
      request: {
        input: [{ text: "Working request" }],
        mode: "execute",
        role: "primary",
      },
      run_id: "browser-active-run",
      revision: 1,
      last_sequence: 0,
      snapshot: {
        run: {
          run_id: "browser-active-run",
          session_id: "browser-session",
          title: "Working request",
          status: "running",
          role: "primary",
          mode: "execute",
          created_at: timestamp,
          updated_at: timestamp,
          last_sequence: 0,
        },
        pending_interactions: [],
      },
    });
  }
  const cancellationIds: string[] = [];
  const credentialWrites: { path: string; body: Record<string, unknown> }[] =
    [];
  let personalCredential = false;
  const credentialStores: Record<"personal" | "project", CredentialSnapshot> = {
    personal: { revision: 0, credentials: [], connections: [] },
    project: { revision: 0, credentials: [], connections: [] },
  };
  const connectionActions: string[] = [];
  const settingsSubmissions: Record<string, unknown>[] = [];
  let projectSettings: ChatSettingsSnapshot = {
    revision: 3,
    providers: {
      profiles: {
        company: {
          kind: "openai_responses",
          baseUrl: "https://models.example/v1",
          credentialReference: null,
          timeoutMs: 300000,
          generationTimeoutMs: 1200000,
        },
      },
    },
    models: {
      profiles: {
        careful: {
          providerProfile: "company",
          model: "configured-model",
          contextWindowTokens: 128000,
          maxOutputTokens: 16000,
          capabilities: {
            toolCalls: "on",
            streaming: "on",
            imageInputs: "off",
          },
          reasoningEffort: "high",
        },
      },
      roles: { primary: "careful" },
    },
    mcp: {
      oauthCredentialStore: "encrypted_state",
      servers: {
        docs: {
          transport: "streamable_http",
          url: "https://docs.example/mcp",
          allowedTools: ["read", "search"],
          oauth: {
            clientId: "registered-client",
            callbackPort: 8787,
            scopes: ["read"],
          },
        },
      },
    },
    provider_authentication: { company: "shared" },
    mcp_authentication: { docs: "personal" },
    credential_status: { providers: { company: true }, mcp: {} },
    labels: { providers: {}, models: {} },
    ...options.settingsOverride,
  };
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.route("**/api/**", async (route) => {
    const request = route.request(),
      url = new URL(request.url()),
      path = url.pathname;
    const json = (value: unknown) =>
      route.fulfill({ status: 200, json: value });
    const storeRoute =
      /^\/api\/projects\/browser-project\/chat\/credential-store\/(personal|project)(?:\/(.*))?$/.exec(
        path,
      );
    if (storeRoute) {
      const scope = storeRoute[1] as "personal" | "project";
      const store = credentialStores[scope];
      store.connections =
        scope === "personal"
          ? [
              {
                id: "personal-key",
                kind: "provider",
                label: "Personal provider",
                endpoint: "https://personal.example/v1",
                credential_binding: "synthetic-reviewed-binding",
                credential_id: store.connections[0]?.credential_id ?? null,
              },
            ]
          : Object.entries(projectSettings.providers.profiles)
              .filter(
                ([id]) =>
                  projectSettings.provider_authentication[id] === "shared",
              )
              .map(([id, profile]) => ({
                id,
                kind: "provider",
                label: projectSettings.labels?.providers[id] ?? id,
                endpoint: profile.baseUrl ?? "",
                credential_binding: "project-binding-" + id,
                credential_id:
                  store.connections.find((row) => row.id === id)
                    ?.credential_id ?? null,
              }));
      if (request.method() === "GET") return json(store);
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON();
      expect(body.revision).toBe(store.revision);
      const suffix = storeRoute[2];
      if (suffix?.startsWith("attachments/")) {
        const [, kind, id] = suffix.split("/");
        const row = store.connections.find(
          (row) => row.id === id && row.kind === kind,
        )!;
        expect(body.credential_binding).toBe(row.credential_binding);
        row.credential_id = body.credential_id;
      } else if (request.method() === "DELETE") {
        store.credentials = store.credentials.filter(
          (row) => row.id !== suffix,
        );
        for (const row of store.connections)
          if (row.credential_id === suffix) row.credential_id = null;
      } else {
        const id = suffix ?? `${scope}-key-${store.revision}`;
        credentialWrites.push({ path, body });
        store.credentials = [
          ...store.credentials.filter((row) => row.id !== id),
          {
            id,
            label: body.label,
            kind: body.kind,
            legacy: false,
            used_by: [],
          },
        ];
        if (body.attachment) {
          const row = store.connections.find(
            (row) =>
              row.id === body.attachment.connection &&
              row.kind === body.attachment.kind,
          )!;
          expect(body.attachment.credential_binding).toBe(
            row.credential_binding,
          );
          row.credential_id = id;
        }
      }
      for (const credential of store.credentials)
        credential.used_by = store.connections.filter(
          (row) => row.credential_id === credential.id,
        );
      personalCredential = credentialStores.personal.connections.some((row) =>
        Boolean(row.credential_id),
      );
      store.revision++;
      return json({ revision: store.revision });
    }
    if (request.method() === "GET") {
      if (path === `${base}/chat/accounts`)
        return json({
          accounts: [
            {
              id: "chatgpt",
              label: "ChatGPT / Codex",
              status: "setup_required",
              can_connect: false,
              reason:
                "Personal ChatGPT subscription linking is not configured on this server.",
            },
          ],
        });
      if (path === "/api/me")
        return json(
          options.administer
            ? {
                ...identity,
                memberships: identity.memberships.map((membership) => ({
                  ...membership,
                  permissions: ["read", "execute", "control", "administer"],
                })),
              }
            : identity,
        );
      if (path === "/api/auth/config")
        return json({ local_enabled: true, oidc: null });
      if (path === "/api/settings")
        return json({
          classification: {
            enabled: false,
            text: "",
            tone: "neutral",
            position: "top",
          },
        });
      if (path === "/api/dashboard")
        return json({
          generated_at: timestamp,
          counts: {
            projects: 1,
            hosts: host ? 1 : 0,
            agents: 1,
            online_agents: 1,
            threads: created ? 1 : 0,
            active_runs: 0,
            queued_tasks: 0,
            failed_runs: 0,
          },
          recent_threads: created
            ? [{ thread, project_name: identity.projects[0]!.name }]
            : [],
          activity: [],
        });
      if (path === `${base}/nodes`)
        return json({ nodes: [runtimeNode], next_cursor: null });
      if (path === `${base}/nodes/${nodeId}`) return json(runtimeNode);
      if (path === `${base}/hosts`)
        return json({ hosts: host ? [host] : [], next_cursor: null });
      if (host && path === `${base}/hosts/browser-host`) return json({ host });
      if (path === `${base}/tasks`)
        return json({ tasks: [], next_cursor: null });
      if (path === `${base}/chat`)
        return json({ enabled: true, node: chatStarted ? runtimeNode : null });
      if (path === `${base}/chat/providers`)
        return json({
          providers: options.administer
            ? [
                {
                  id: "personal-key",
                  label: "Personal provider",
                  authentication: "personal",
                  authenticated: personalCredential,
                  requires_credential: true,
                  endpoint: "https://personal.example/v1",
                  credential_binding: "synthetic-reviewed-binding",
                },
              ]
            : [],
        });
      if (
        path === `${base}/chat/settings/provider-presets` &&
        options.administer
      )
        return json({
          availability: Object.fromEntries(
            (
              options.providerPresets ?? [{ id: "openai" }, { id: "anthropic" }]
            ).map(({ id }) => [
              id,
              id === "codex"
                ? {
                    status: "deployment_required",
                    reason:
                      "Codex sign-in is managed by your deployment administrator.",
                  }
                : id === "ollama" || id === "lmstudio"
                  ? {
                      status: "configure_endpoint",
                      reason:
                        "Configure an endpoint reachable from the cloud host.",
                    }
                  : id.startsWith("custom-")
                    ? {
                        status: "configure_endpoint",
                        reason: "Enter the provider endpoint.",
                      }
                    : {
                        status: "available",
                        reason: "Available for this project.",
                      },
            ]),
          ),
          presets: options.providerPresets ?? [
            {
              id: "openai",
              label: "OpenAI",
              protocol: "responses",
              baseUrl: "https://api.openai.com/v1",
              credentialEnv: "OPENAI_API_KEY",
            },
            {
              id: "anthropic",
              label: "Anthropic",
              protocol: "anthropic_messages",
              baseUrl: "https://api.anthropic.com/v1",
              credentialEnv: "ANTHROPIC_API_KEY",
            },
          ],
        });
      if (
        path === `${base}/chat/settings` &&
        (options.administer || options.globalAdmin)
      )
        return json(projectSettings);
      if (path === `${base}/chat/threads`)
        return json({ threads: created ? [thread] : [], next_cursor: null });
      if (path === `${base}/chat/connections`)
        return json({
          connections: [
            {
              id: "personal",
              label: "Personal source",
              transport: "streamable_http",
              authentication: "personal",
              authenticated: personalConnected,
              oauth: true,
              allowed_tools: ["read"],
            },
            {
              id: "project",
              label: "Project source",
              transport: "streamable_http",
              authentication: "shared",
              authenticated: true,
              oauth: false,
              allowed_tools: ["search"],
            },
          ],
        });
      if (path === `${base}/threads`)
        return json({ threads: created ? [thread] : [], next_cursor: null });
      if (path === `${base}/threads/browser-thread`)
        return json({ thread, messages, tasks });
      if (path === `${base}/threads/browser-thread/events`)
        return route.fulfill({
          status: 200,
          contentType: "text/event-stream",
          body: ": synthetic browser fixture\n\n",
        });
    }
    if (
      request.method() === "PUT" &&
      path === `${base}/chat/settings` &&
      options.administer
    ) {
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON();
      expect(body.revision).toBe(projectSettings.revision);
      expect(body).not.toHaveProperty("credential_status");
      settingsSubmissions.push(body);
      projectSettings = {
        ...projectSettings,
        ...body,
        revision: projectSettings.revision + 1,
      };
      return json({ revision: projectSettings.revision });
    }
    if (request.method() === "POST") {
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON();
      if (path === "/api/projects" && options.globalAdmin) {
        identity.projects.push({
          ...body,
          archived: false,
          revision: 1,
          created_at: timestamp,
          updated_at: timestamp,
        });
        return json({ project: identity.projects.at(-1) });
      }
      if (path === `${base}/tasks/browser-active-task/cancel`) {
        cancellationIds.push(body.mutation_id);
        tasks[0]!.snapshot!.run.status = "cancelled";
        thread.active_task_id = null;
        return json({
          command: {
            command_id: "browser-cancel",
            reply: { kind: "completed" },
          },
        });
      }
      if (
        options.administer &&
        path === `${base}/chat/settings/providers/company/models`
      ) {
        expect(body).toEqual({ revision: projectSettings.revision });
        return json({
          models: [
            {
              id: "catalog-reasoner",
              display_name: "Catalog Reasoner",
              context_window_tokens: 200000,
              max_output_tokens: 30000,
              tool_calls: true,
              image_inputs: true,
              streaming: true,
              server_compaction: true,
              supported_reasoning_efforts: ["low", "high"],
            },
          ],
        });
      }
      if (
        options.administer &&
        path === `${base}/chat/settings/providers/company/credential`
      ) {
        expect(body.revision).toBe(projectSettings.revision);
        credentialWrites.push({ path, body });
        projectSettings.credential_status.providers.company = true;
        projectSettings.revision++;
        return json({ revision: projectSettings.revision });
      }
      if (
        options.administer &&
        path === `${base}/chat/providers/personal-key/credential`
      ) {
        expect(body.credential_binding).toBe("synthetic-reviewed-binding");
        credentialWrites.push({ path, body });
        personalCredential = true;
        return json({ authenticated: true });
      }
      if (
        options.administer &&
        path === `${base}/chat/providers/personal-key/disconnect`
      ) {
        personalCredential = false;
        return json({ authenticated: false });
      }
      if (path === `${base}/chat/connect`) {
        chatStarted = true;
        return json({ enabled: true, node: runtimeNode });
      }
      if (path === `${base}/chat/connections/personal/auth`) {
        connectionActions.push("auth");
        return json({
          authorization_url:
            "https://identity.example/authorize?state=synthetic-flow",
        });
      }
      if (path === `${base}/chat/connections/personal/disconnect`) {
        connectionActions.push("disconnect");
        personalConnected = false;
        return json({});
      }
      if (path === `${base}/nodes/${nodeId}/resources`) {
        expect(body.connection_id).toBe("browser-connection");
        if (body.operation.operation === "context")
          return json({
            connection_id: "browser-connection",
            kind: "result",
            value: { capabilities },
          });
        if (body.operation.operation === "model_catalog")
          return json({
            connection_id: "browser-connection",
            kind: "result",
            value: {
              model_profiles: [
                {
                  profile: "fast",
                  model: "Fast browser model",
                  provider: "Test provider",
                  provider_profile: "synthetic",
                  reasoning_effort: "medium",
                  default_roles: [],
                },
                {
                  profile: "careful",
                  model: "Careful browser model",
                  provider: "Test provider",
                  provider_profile: "synthetic",
                  reasoning_effort: "high",
                  default_roles: ["primary"],
                },
              ],
            },
          });
      }
      if (
        path === `${base}/threads` ||
        path === `${base}/chat/threads` ||
        path === `${base}/threads/browser-thread/messages`
      ) {
        if (path.endsWith("/messages")) {
          expect(body.expected_revision).toBe(thread.revision);
          thread.revision++;
        } else {
          if (path.endsWith("/chat/threads")) {
            expect(body).not.toHaveProperty("node_id");
            thread.owner_user_id = me.user.id;
          } else expect(body.node_id).toBe(nodeId);
          created = true;
        }
        submissions.push({ path, ...body });
        messages.push({
          message_id: `browser-message-${submissions.length}`,
          role: "user",
          text: body.request.input[0].text,
          created_at: timestamp,
        });
        return json({ thread });
      }
    }
    unexpected.push(`${request.method()} ${url.pathname}${url.search}`);
    return route.fulfill({
      status: 500,
      json: { error: "unhandled_browser_fixture_request" },
    });
  });
  return {
    nodeId,
    submissions,
    connectionActions,
    settingsSubmissions,
    credentialWrites,
    cancellationIds,
    authenticatePersonal: () => {
      personalConnected = true;
    },
    verify: (expectedConsoleErrors: string[] = []) => {
      expect(unexpected).toEqual([]);
      expect(errors).toEqual(expectedConsoleErrors);
    },
  };
}
