import { expect, test, type Page } from "@playwright/test";
import type {
  Authentication,
  ChatSettingsSnapshot,
  ChatSettingsWrite,
  InheritedSection,
  SettingsInheritanceSnapshot,
} from "../../src/chat-settings";
import type { CredentialSnapshot } from "../../src/CredentialStoreSettings";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";

const project = "browser-project";
const api = `/api/projects/${project}`;
const globalPath = "/api/admin/hosted-settings";
const projectPath = `${api}/chat/settings`;
const globalStore = "/api/admin/credential-store";
const clone = <T>(value: T): T => structuredClone(value);
// Each journey publishes several resources, changes actor, and submits from a fresh app load.
test.describe.configure({ timeout: 90_000 });
type Actor = "instance-admin" | "project-admin" | "member";
type Kind = "provider" | "mcp" | "search";
type StoreScope = "personal" | "project" | "global";

function emptySettings(): ChatSettingsSnapshot {
  return {
    revision: 1,
    labels: { providers: {}, models: {}, search: {} },
    providers: { profiles: {} },
    models: { profiles: {}, roles: {} },
    mcp: { servers: {} },
    search: { profiles: {}, roles: {} },
    provider_authentication: {},
    mcp_authentication: {},
    search_authentication: {},
    variables: {},
    connection_network_policy: { mode: "allow_all", allowed_origins: [] },
    credential_status: { providers: {}, mcp: {}, search: {} },
  };
}

/** Synthetic server projections exercise the real renderer, not runtime inheritance. */
async function onboardingFixture(page: Page) {
  const base = await browserFixture(page);
  let actor: Actor = "instance-admin";
  let global = emptySettings();
  let local = emptySettings();
  const configurationWrites: {
    actor: Actor;
    path: string;
    body: ChatSettingsWrite;
  }[] = [];
  const credentialWrites: {
    actor: Actor;
    scope: StoreScope;
    kind: Kind;
    id: string;
  }[] = [];
  const stores: Record<StoreScope, CredentialSnapshot> = {
    personal: { revision: 1, credentials: [], connections: [] },
    project: { revision: 1, credentials: [], connections: [] },
    global: { revision: 1, credentials: [], connections: [] },
  };
  const attached: Record<StoreScope, Map<string, string>> = {
    personal: new Map(),
    project: new Map(),
    global: new Map(),
  };
  let nextProjectSave:
    | ((
        body: ChatSettingsWrite,
        current: ChatSettingsSnapshot,
      ) => ChatSettingsSnapshot)
    | undefined;
  const binding = (kind: Kind, id: string) =>
    `reviewed-${kind}-${id}-generation-1`;
  const key = (kind: Kind, id: string) => `${kind}:${id}`;
  function auth(kind: Kind, id: string): Authentication {
    return (
      (kind === "provider"
        ? local.provider_authentication[id]
        : kind === "mcp"
          ? local.mcp_authentication[id]
          : local.search_authentication?.[id]) ?? "personal"
    );
  }
  function authenticated(kind: Kind, id: string) {
    return (
      auth(kind, id) === "none" ||
      (auth(kind, id) === "personal"
        ? attached.personal.has(key(kind, id))
        : attached.project.has(key(kind, id)) ||
          attached.global.has(key(kind, id)))
    );
  }
  function destinations() {
    return [
      ...Object.entries(local.providers.profiles).map(([id, value]) => ({
        id,
        kind: "provider" as const,
        label: local.labels?.providers[id] ?? id,
        endpoint: value.baseUrl ?? "",
      })),
      ...Object.entries(local.mcp.servers).map(([id, value]) => ({
        id,
        kind: "mcp" as const,
        label: id,
        endpoint: value.url,
      })),
      ...Object.entries(local.search?.profiles ?? {}).map(([id, value]) => ({
        id,
        kind: "search" as const,
        label: local.labels?.search?.[id] ?? id,
        endpoint: value.endpoint ?? "https://serpapi.com",
      })),
    ];
  }
  function inheritedSnapshot(): SettingsInheritanceSnapshot {
    const entries: Record<InheritedSection, string[]> = {
      providers: Object.keys(global.providers.profiles),
      models: Object.keys(global.models.profiles),
      mcp: Object.keys(global.mcp.servers),
      search: Object.keys(global.search?.profiles ?? {}),
      variables: Object.keys(global.variables ?? {}),
      roles: [
        ...Object.keys(global.models.roles),
        ...Object.keys(global.search?.roles ?? {}).map(
          (role) => `search:${role}`,
        ),
      ],
    };
    const empty = {
      providers: [],
      models: [],
      mcp: [],
      search: [],
      variables: [],
      roles: [],
    };
    return {
      global_revision: global.revision,
      global_entries: entries,
      overrides: clone(empty),
      disabled: clone(empty),
      provenance: Object.fromEntries(
        Object.entries(entries).map(([section, ids]) => [
          section,
          Object.fromEntries(ids.map((id) => [id, "global"])),
        ]),
      ) as SettingsInheritanceSnapshot["provenance"],
    };
  }
  function inheritPublishedDefinitions() {
    local = {
      ...clone(global),
      revision: local.revision,
      inheritance: inheritedSnapshot(),
    };
  }
  function storeSnapshot(
    scope: StoreScope,
    selectedProject: boolean,
  ): CredentialSnapshot {
    const store = stores[scope];
    const connections =
      scope === "global" && !selectedProject
        ? []
        : destinations()
            .filter(
              (row) =>
                auth(row.kind, row.id) ===
                (scope === "personal" ? "personal" : "shared"),
            )
            .map((row) => ({
              ...row,
              ...(scope === "global" ? { project_id: project } : {}),
              credential_binding: binding(row.kind, row.id),
              credential_id: attached[scope].get(key(row.kind, row.id)) ?? null,
              credential_source: attached.global.has(key(row.kind, row.id))
                ? ("global" as const)
                : scope === "personal"
                  ? ("personal" as const)
                  : ("project" as const),
            }));
    return {
      revision: store.revision,
      credentials: store.credentials.map((credential) => ({
        ...credential,
        used_by: connections.filter(
          (row) => row.credential_id === credential.id,
        ),
      })),
      connections,
    };
  }
  await page.route("**/api/**", async (route) => {
    const request = route.request(),
      url = new URL(request.url()),
      path = url.pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (path === "/api/me")
      return respond({
        user: {
          id: actor === "member" ? "browser-user" : actor,
          display_name: actor === "member" ? "New project member" : actor,
          email: null,
          active: true,
          is_admin: actor === "instance-admin",
          revision: 1,
          created_at: "2026-10-23T00:00:00Z",
          updated_at: "2026-10-23T00:00:00Z",
        },
        projects: [
          {
            id: project,
            name: "Onboarding project",
            description: "Synthetic inherited settings",
            parent_project_id: null,
            archived: false,
            revision: 1,
            created_at: "",
            updated_at: "",
          },
        ],
        memberships: [
          {
            project_id: project,
            user_id: actor === "member" ? "browser-user" : actor,
            subject: actor,
            role: actor === "member" ? "operator" : "project_admin",
            permissions:
              actor === "member"
                ? ["read", "execute", "control"]
                : ["read", "execute", "control", "administer"],
          },
        ],
      });
    if (path === globalPath || path === projectPath) {
      expect(actor).not.toBe("member");
      if (path === globalPath) expect(actor).toBe("instance-admin");
      const current = path === globalPath ? global : local;
      if (request.method() === "GET") return respond(current);
      expect(request.method()).toBe("PUT");
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON() as ChatSettingsWrite;
      expect(body.revision).toBe(current.revision);
      expect(body).not.toHaveProperty("credential_status");
      expect(JSON.stringify(body)).not.toContain("synthetic-private-token");
      configurationWrites.push({ actor, path, body });
      if (path === globalPath) {
        global = {
          ...current,
          ...clone(body),
          revision: current.revision + 1,
        } as ChatSettingsSnapshot;
        inheritPublishedDefinitions();
      } else {
        const apply = nextProjectSave;
        nextProjectSave = undefined;
        local =
          apply?.(body, current) ??
          ({
            ...current,
            ...clone(body),
            revision: current.revision + 1,
            inheritance: current.inheritance,
          } as ChatSettingsSnapshot);
      }
      return respond({
        revision: path === globalPath ? global.revision : local.revision,
      });
    }
    if (
      path === `${globalPath}/provider-presets` ||
      path === `${projectPath}/provider-presets`
    )
      return respond({ presets: [] });
    const storeMatch = new RegExp(
      `^${api}/chat/credential-store/(personal|project)(?:/(.*))?$`,
      "u",
    ).exec(path);
    if (
      path === globalStore ||
      path.startsWith(`${globalStore}/`) ||
      storeMatch
    ) {
      const scope: StoreScope = storeMatch
        ? (storeMatch[1] as "personal" | "project")
        : "global";
      if (scope === "global") expect(actor).toBe("instance-admin");
      if (scope === "project") expect(actor).not.toBe("member");
      if (request.method() === "GET")
        return respond(
          storeSnapshot(scope, url.searchParams.get("project") === project),
        );
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON();
      expect(body.revision).toBe(stores[scope].revision);
      const suffix = storeMatch
        ? (storeMatch[2] ?? "")
        : path.slice(globalStore.length + 1);
      if (suffix.startsWith("attachments/")) {
        const pieces = suffix.split("/");
        const kind = pieces.at(-2) as Kind,
          id = pieces.at(-1)!;
        expect(body.credential_binding).toBe(binding(kind, id));
        attached[scope].set(key(kind, id), body.credential_id);
        credentialWrites.push({ actor, scope, kind, id });
      } else {
        expect(body.secret).toBe("synthetic-private-token");
        const credentialId = `${scope}-secret-${stores[scope].revision}`;
        stores[scope].credentials.push({
          id: credentialId,
          label: body.label,
          kind: body.kind,
          legacy: false,
          used_by: [],
        });
        if (body.attachment) {
          const {
            kind,
            connection: id,
            credential_binding: reviewed,
          } = body.attachment as {
            kind: Kind;
            connection: string;
            credential_binding: string;
          };
          expect(reviewed).toBe(binding(kind, id));
          if (scope === "global")
            expect(body.attachment.project_id).toBe(project);
          attached[scope].set(key(kind, id), credentialId);
          credentialWrites.push({ actor, scope, kind, id });
        }
      }
      stores[scope].revision++;
      return respond({ revision: stores[scope].revision });
    }
    const personalConnection = new RegExp(
      `^${api}/chat/(providers|connections)/([^/]+)/credential$`,
      "u",
    ).exec(path);
    if (personalConnection) {
      expect(actor).toBe("member");
      expect(request.method()).toBe("POST");
      const kind: Kind =
          personalConnection[1] === "providers" ? "provider" : "mcp",
        id = personalConnection[2]!;
      const body = request.postDataJSON();
      expect(auth(kind, id)).toBe("personal");
      expect(body).toEqual({
        secret: "synthetic-private-token",
        credential_binding: binding(kind, id),
      });
      attached.personal.set(key(kind, id), `private-${id}`);
      credentialWrites.push({ actor, scope: "personal", kind, id });
      return respond({ authenticated: true });
    }
    if (path === `${api}/chat/providers`)
      return respond({
        providers: Object.entries(local.providers.profiles).map(
          ([id, value]) => ({
            id,
            label: local.labels?.providers[id] ?? id,
            authentication: auth("provider", id),
            authenticated: authenticated("provider", id),
            requires_credential: auth("provider", id) !== "none",
            endpoint: value.baseUrl,
            credential_binding: binding("provider", id),
          }),
        ),
      });
    if (path === `${api}/chat/connections`)
      return respond({
        connections: Object.entries(local.mcp.servers).map(([id, value]) => ({
          id,
          label: id,
          transport: "streamable_http",
          authentication: auth("mcp", id),
          authenticated: authenticated("mcp", id),
          oauth: false,
          allowed_tools: value.allowedTools,
          credential_binding: binding("mcp", id),
          endpoint: value.url,
        })),
      });
    if (request.method() === "POST" && path.endsWith("/resources")) {
      const body = request.postDataJSON();
      if (body.operation.operation === "model_catalog")
        return respond({
          connection_id: "browser-connection",
          kind: "result",
          value: {
            model_profiles: Object.entries(local.models.profiles).map(
              ([id, value]) => ({
                profile: id,
                label: local.labels?.models[id] ?? id,
                model: value.model,
                provider:
                  local.labels?.providers[value.providerProfile] ??
                  value.providerProfile,
                provider_profile: value.providerProfile,
                reasoning_effort: "high",
                default_roles: Object.entries(local.models.roles)
                  .filter(([, profile]) => profile === id)
                  .map(([role]) => role),
              }),
            ),
          },
        });
    }
    if (request.method() === "POST" && path === `${api}/chat/threads`) {
      expect(actor).toBe("member");
      expect(authenticated("provider", "org-api")).toBe(true);
      expect(authenticated("mcp", "org-docs")).toBe(true);
    }
    return route.fallback();
  });
  return {
    ...base,
    configurationWrites,
    credentialWrites,
    get global() {
      return global;
    },
    get local() {
      return local;
    },
    set local(value: ChatSettingsSnapshot) {
      local = value;
    },
    as(next: Actor) {
      actor = next;
    },
    nextProjectSave(apply: NonNullable<typeof nextProjectSave>) {
      nextProjectSave = apply;
    },
    verify() {
      base.verify();
      expect(
        credentialWrites.every(
          (write) => write.scope !== "personal" || write.actor === "member",
        ),
      ).toBe(true);
    },
  };
}

async function choose(page: Page, name: string, option: string) {
  await page
    .getByRole("combobox", { name: new RegExp(`^${name}(?:\\s|$)`, "u") })
    .click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

async function publishCatalog(page: Page, shared: boolean) {
  await page.goto("/admin/providers");
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  await page
    .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
    .fill("Organization API");
  await page
    .getByRole("textbox", { name: /^Profile ID(?:\s|$)/u })
    .fill("org-api");
  await page
    .getByRole("textbox", { name: /^Endpoint URL(?:\s|$)/u })
    .fill("https://models.example/v1");
  if (shared)
    await choose(
      page,
      "Authentication",
      "Shared credential (explicit assignment)",
    );
  await page.getByRole("button", { name: "Add provider", exact: true }).click();
  await expect(page).toHaveURL(/\/admin\/providers$/u);
  await page.goto("/admin/mcp");
  await page
    .getByRole("button", { name: "Add MCP connection", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: /^Profile ID(?:\s|$)/u })
    .fill("org-docs");
  await page
    .getByRole("textbox", { name: "MCP server URL", exact: true })
    .fill("https://docs.example/mcp");
  await page
    .getByRole("textbox", { name: "Allowed tools", exact: true })
    .fill("search, read_*");
  if (shared)
    await choose(
      page,
      "Authentication",
      "Shared credential (explicit assignment)",
    );
  await page
    .getByRole("button", { name: "Add MCP connection", exact: true })
    .click();
  await expect(page).toHaveURL(/\/admin\/mcp$/u);
  await page.goto("/admin/models");
  await page.getByRole("button", { name: "Add model", exact: true }).click();
  await page
    .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
    .fill("Organization reasoner");
  await page
    .getByRole("textbox", { name: /^Profile ID(?:\s|$)/u })
    .fill("org-reasoner");
  await choose(page, "Provider profile", "Organization API");
  await page
    .getByRole("textbox", { name: /^Model identifier(?:\s|$)/u })
    .fill("organization-reasoner-v1");
  await page
    .getByRole("spinbutton", { name: /^Context window \(tokens\)/u })
    .fill("128000");
  await page
    .getByRole("spinbutton", { name: /^Maximum output \(tokens\)/u })
    .fill("16000");
  await page.getByRole("button", { name: "Add model", exact: true }).click();
  await expect(page).toHaveURL(/\/admin\/models$/u);
  await page.goto("/admin/routing/edit");
  await choose(
    page,
    "Research planner model",
    "Organization reasoner · organization-reasoner-v1",
  );
  await page.getByRole("button", { name: "Save routing", exact: true }).click();
  await expect(page).toHaveURL(/\/admin\/routing$/u);
}

async function submitWithInheritedModel(
  page: Page,
  fixture: Awaited<ReturnType<typeof onboardingFixture>>,
) {
  await page.goto(`/chat?project=${project}`);
  await expect(
    page.getByRole("link", { name: "Control Plane Admin", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "Project Admin", exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Start chat", exact: true }).click();
  const picker = page.getByRole("button", {
    name: /Choose model and reasoning:/u,
  });
  await picker.click();
  const search = page.getByRole("combobox", {
    name: "Search models",
    exact: true,
  });
  await search.fill("organization-reasoner-v1");
  await search.press("End");
  await search.press("Enter");
  await page.keyboard.press("Escape");
  await page
    .getByRole("textbox", { name: "Message Colossus", exact: true })
    .fill("Use my inherited project configuration");
  await page
    .getByRole("textbox", { name: "Message Colossus", exact: true })
    .press("Enter");
  await expect.poll(() => fixture.submissions.length).toBe(1);
  expect(fixture.submissions[0]).toMatchObject({
    path: `${api}/chat/threads`,
    request: {
      mode: "execute",
      model_profile: "org-reasoner",
      role: "primary",
    },
  });
  expect(JSON.stringify(fixture.submissions)).not.toContain(
    "synthetic-private-token",
  );
  await expect(page).toHaveURL(
    /\/projects\/browser-project\/chat\/browser-thread$/u,
  );
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
}

test("a fresh member privately connects inherited provider and MCP definitions, then uses the inherited model", async ({
  page,
}) => {
  const fixture = await onboardingFixture(page);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await publishCatalog(page, false);
  expect(fixture.configurationWrites).toHaveLength(4);
  fixture.as("project-admin");
  await page.goto(`/projects/${project}/settings/providers`);
  await expect(
    page.getByRole("table", { name: "Configured providers", exact: true }),
  ).toContainText("Inherited from Control Plane");
  await expect(
    page.getByRole("button", { name: "Edit org-api", exact: true }),
  ).toHaveText("Customize");
  await page.goto(`/projects/${project}/settings/models`);
  await expect(
    page.getByRole("table", { name: "Configured models", exact: true }),
  ).toContainText("Organization reasoner");
  fixture.as("member");
  await page.goto(`/projects/${project}/chat/connections`);
  for (const name of ["Organization API provider", "org-docs"]) {
    const connection = page.getByRole("region", { name, exact: true });
    await expect(connection).toContainText("Not connected");
    await connection
      .locator('input[aria-label="Your API key or token"]')
      .fill("synthetic-private-token");
    await connection
      .getByRole("button", { name: "Save personal credential", exact: true })
      .click();
    await expect(connection).toContainText("Available");
    await expect(connection.locator('input[type="password"]')).toHaveValue("");
  }
  expect(fixture.credentialWrites).toEqual([
    { actor: "member", scope: "personal", kind: "provider", id: "org-api" },
    { actor: "member", scope: "personal", kind: "mcp", id: "org-docs" },
  ]);
  await submitWithInheritedModel(page, fixture);
  await page.screenshot({
    path: "output/playwright/settings-onboarding-personal-inherited-model.png",
  });
  fixture.verify();
});

for (const scope of ["project", "global"] as const) {
  test(`a fresh member uses ${scope}-assigned inherited connections without entering a token`, async ({
    page,
  }) => {
    const fixture = await onboardingFixture(page);
    await page.setViewportSize({ width: 1280, height: 1000 });
    await publishCatalog(page, true);
    fixture.as(scope === "global" ? "instance-admin" : "project-admin");
    const base =
      scope === "global"
        ? `/admin/credentials?project=${project}`
        : `/projects/${project}/settings/credentials`;
    await page.goto(base);
    await page
      .getByRole("button", { name: "Add credential", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
      .fill("Assigned organization key");
    await choose(page, "Credential connection", "Provider: Organization API");
    await page
      .locator('input[aria-label="API key or token"]')
      .fill("synthetic-private-token");
    await page
      .getByRole("button", { name: "Save credential", exact: true })
      .click();
    await expect(
      page.getByRole("button", {
        name: "Attach existing credential",
        exact: true,
      }),
    ).toBeEnabled();
    await page
      .getByRole("button", { name: "Attach existing credential", exact: true })
      .click();
    await choose(page, "Saved credential", "Assigned organization key");
    await choose(page, "Credential connection", "MCP: org-docs");
    await page
      .getByRole("button", { name: "Attach credential", exact: true })
      .click();
    await expect.poll(() => fixture.credentialWrites.length).toBe(2);
    await expect(
      page.getByRole("button", {
        name: "Attach existing credential",
        exact: true,
      }),
    ).toBeEnabled();
    fixture.as("member");
    await page.goto(`/projects/${project}/chat/connections`);
    for (const name of ["Organization API provider", "org-docs"]) {
      const connection = page.getByRole("region", { name, exact: true });
      await expect(connection).toContainText("Project connection · Available");
      await expect(connection.getByRole("button")).toHaveCount(0);
    }
    await expect(page.locator('input[type="password"]')).toHaveCount(0);
    await submitWithInheritedModel(page, fixture);
    expect(
      fixture.credentialWrites.every(
        (write) => write.scope === scope && write.actor !== "member",
      ),
    ).toBe(true);
    expect(
      fixture.configurationWrites.every(
        (write) => write.actor === "instance-admin",
      ),
    ).toBe(true);
    fixture.verify();
  });
}

for (const kind of ["searxng", "serp_api"] as const) {
  test(
    "a fresh member can attach a private credential to an inherited " +
      kind +
      " Search service without project settings authority",
    async ({ page }) => {
      const fixture = await onboardingFixture(page);
      await page.setViewportSize({ width: 1280, height: 1000 });
      await page.goto("/admin/search");
      await page
        .getByRole("button", { name: "Add search service", exact: true })
        .click();
      await page
        .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
        .fill("Organization search");
      await page
        .getByRole("textbox", { name: /^Profile ID(?:\s|$)/u })
        .fill("org-search");
      if (kind === "serp_api") {
        await choose(page, "Adapter", "SerpAPI");
        await expect(
          page.getByRole("combobox", {
            name: "Search authentication",
            exact: true,
          }),
        ).toContainText("Personal credential (each user)");
      } else {
        await page
          .getByRole("textbox", { name: /^Authentication header(?:\s|$)/u })
          .fill("X-Organization-Search");
      }
      const endpoint =
        kind === "serp_api"
          ? "https://serpapi.com/search"
          : "https://search.example/search";
      await page
        .getByRole("textbox", { name: /^Endpoint URL/u })
        .fill(endpoint);
      await page
        .getByRole("textbox", { name: "User agent", exact: true })
        .fill("Organization Search/2.0");
      await choose(
        page,
        "Search authentication",
        "Personal credential (each user)",
      );
      await page
        .getByRole("button", { name: "Save search service", exact: true })
        .click();
      await expect(page).toHaveURL(/\/admin\/search$/u);
      expect(fixture.configurationWrites[0]?.body).toMatchObject({
        search_authentication: { "org-search": "personal" },
        search: {
          profiles: {
            "org-search": {
              kind,
              endpoint,
              userAgent: "Organization Search/2.0",
              credentialReference: "host:managed",
            },
          },
        },
      });
      if (kind === "serp_api")
        expect(
          fixture.configurationWrites[0]?.body.search?.profiles["org-search"],
        ).not.toHaveProperty("authHeader");
      else
        expect(
          fixture.configurationWrites[0]?.body.search?.profiles["org-search"]
            ?.authHeader,
        ).toBe("X-Organization-Search");
      fixture.as("project-admin");
      await page.goto(`/projects/${project}/settings/search`);
      await expect(
        page.getByRole("table", {
          name: "Configured search services",
          exact: true,
        }),
      ).toContainText("Organization search");
      fixture.as("member");
      await page.setViewportSize({ width: 390, height: 844 });
      await selectAppearance(page, appearanceCases[1]);
      await page.goto(
        `/settings/credentials/new?project=${project}&kind=search&connection=org-search`,
      );
      await expect(
        page.getByRole("combobox", {
          name: "Credential connection",
          exact: true,
        }),
      ).toContainText("Search: Organization search");
      await page
        .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
        .fill("My private search key");
      await page
        .locator('input[aria-label="API key or token"]')
        .fill("synthetic-private-token");
      await page
        .getByRole("button", { name: "Save credential", exact: true })
        .click();
      await expect(page).toHaveURL(
        new RegExp(`/settings/credentials\\?project=${project}$`, "u"),
      );
      await expect(
        page.getByRole("table", {
          name: "Configured credentials",
          exact: true,
        }),
      ).toContainText("My private search key");
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path:
          kind === "searxng"
            ? "output/playwright/settings-onboarding-private-search-light-large-mobile.png"
            : "output/playwright/settings-onboarding-private-serp-api-search-light-large-mobile.png",
        fullPage: true,
      });
      expect(fixture.credentialWrites).toEqual([
        {
          actor: "member",
          scope: "personal",
          kind: "search",
          id: "org-search",
        },
      ]);
      expect(fixture.configurationWrites).toHaveLength(1);
      await page.goto(`/projects/${project}/settings/search`);
      await expect(
        page.getByText(
          "Your signed-in identity does not have administration access to this project.",
          { exact: true },
        ),
      ).toBeVisible();
      fixture.verify();
    },
  );
}

test("project customization and inheritance reset retain the global catalog while disabled MCP definitions can be restored", async ({
  page,
}) => {
  const fixture = await onboardingFixture(page);
  await page.setViewportSize({ width: 1280, height: 1000 });
  await publishCatalog(page, false);
  const inherited = clone(fixture.local);
  fixture.as("project-admin");
  await page.goto(`/projects/${project}/settings/providers`);
  await page.getByRole("button", { name: "Edit org-api", exact: true }).click();
  await page
    .getByRole("textbox", { name: /^Display label(?:\s|$)/u })
    .fill("Project-specific API");
  fixture.nextProjectSave((body, current) => ({
    ...current,
    ...clone(body),
    revision: current.revision + 1,
    inheritance: {
      ...current.inheritance!,
      overrides: { ...current.inheritance!.overrides, providers: ["org-api"] },
      provenance: {
        ...current.inheritance!.provenance,
        providers: { "org-api": "project" },
      },
    },
  }));
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(
    page.getByRole("table", { name: "Configured providers", exact: true }),
  ).toContainText("Project-specific API");
  expect(fixture.global.labels?.providers["org-api"]).toBe("Organization API");
  const defaults = page.getByRole("region", {
    name: "Inherited configuration",
    exact: true,
  });
  await defaults.locator("summary").click();
  await expect(defaults).toContainText("Project override");
  fixture.nextProjectSave((body, current) => {
    expect(body.inheritance?.reset).toEqual({ providers: ["org-api"] });
    return { ...clone(inherited), revision: current.revision + 1 };
  });
  await defaults
    .getByRole("button", { name: "Reset to Control Plane", exact: true })
    .click();
  await expect(
    page.getByRole("table", { name: "Configured providers", exact: true }),
  ).toContainText("Organization API");
  await expect(
    page.getByRole("table", { name: "Configured providers", exact: true }),
  ).not.toContainText("Project-specific API");
  await page.goto(`/projects/${project}/settings/mcp`);
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  fixture.nextProjectSave((body, current) => {
    expect(body.mcp.servers).toEqual({});
    return {
      ...current,
      ...clone(body),
      revision: current.revision + 1,
      inheritance: {
        ...current.inheritance!,
        disabled: { ...current.inheritance!.disabled!, mcp: ["org-docs"] },
        provenance: { ...current.inheritance!.provenance, mcp: {} },
      },
    };
  });
  await page
    .getByRole("button", { name: "Save project configuration", exact: true })
    .click();
  const disabled = page.getByRole("region", {
    name: "Inherited configuration",
    exact: true,
  });
  await disabled.locator("summary").click();
  await expect(disabled).toContainText("Disabled in this project");
  expect(fixture.global.mcp.servers["org-docs"]).toBeDefined();
  fixture.as("member");
  await page.goto(`/projects/${project}/chat/connections`);
  await expect(
    page.getByText("No external connections are configured.", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "org-docs", exact: true }),
  ).toHaveCount(0);
  fixture.as("project-admin");
  await page.goto(`/projects/${project}/settings/mcp`);
  const restore = page.getByRole("region", {
    name: "Inherited configuration",
    exact: true,
  });
  await restore.locator("summary").click();
  fixture.nextProjectSave((body, current) => {
    expect(body.inheritance?.reset).toEqual({ mcp: ["org-docs"] });
    return { ...clone(inherited), revision: current.revision + 1 };
  });
  await restore
    .getByRole("button", { name: "Enable inherited definition", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Edit org-docs", exact: true }),
  ).toBeVisible();
  fixture.as("member");
  await page.goto(`/projects/${project}/chat/connections`);
  await expect(
    page.getByRole("region", { name: "org-docs", exact: true }),
  ).toContainText("Not connected");
  expect(
    fixture.configurationWrites.filter(
      (write) => write.actor === "project-admin",
    ),
  ).toHaveLength(4);
  expect(fixture.credentialWrites).toEqual([]);
  fixture.verify();
});
