import { expect, test, type Page } from "@playwright/test";
import type {
  ChatSettingsSnapshot,
  ChatSettingsWrite,
  SettingsInheritanceSnapshot,
} from "../../src/chat-settings";
import type { CredentialSnapshot } from "../../src/CredentialStoreSettings";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

const project = "browser-project";
const projectSettings = `/api/projects/${project}/chat/settings`;
const globalSettings = "/api/admin/hosted-settings";
const globalSecretStore = "/api/admin/credential-store";
const clone = <T>(value: T): T => structuredClone(value);

function settings(): ChatSettingsSnapshot {
  return {
    revision: 5,
    labels: { providers: {}, models: {}, search: {} },
    providers: { profiles: {} },
    models: { profiles: {}, roles: {} },
    mcp: { servers: {} },
    provider_authentication: {},
    mcp_authentication: {},
    search: { profiles: {}, roles: {} },
    search_authentication: {},
    variables: { REGION: "global-east" },
    credential_status: { providers: {}, mcp: {}, search: {} },
    observability: {
      enabled: true,
      serviceName: "control-plane",
      resourceAttributes: { environment: "browser-fixture" },
      traces: { enabled: true, sampleRatio: 1 },
      metrics: { enabled: false, exportIntervalMs: 60000 },
      logs: {
        otlp: false,
        stdoutJson: false,
        journalPayloads: "disabled",
        acknowledgeSensitiveContent: false,
      },
      otlp: {
        endpoint: "https://collector.example:4317",
        protocol: "grpc",
        timeoutMs: 10000,
        acknowledgeInsecureTransport: false,
      },
    },
    telemetry: { state: "active", configured_revision: 5, applied_revision: 5 },
  };
}

function inheritance(): SettingsInheritanceSnapshot {
  const sections = {
    providers: [],
    models: [],
    mcp: [],
    search: [],
    variables: [],
    roles: [],
  };
  return {
    global_revision: 5,
    overrides: clone(sections),
    global_entries: { ...clone(sections), variables: ["REGION"] },
    disabled: clone(sections),
    provenance: {
      providers: {},
      models: {},
      mcp: {},
      search: {},
      variables: { REGION: "global" },
      roles: {},
    },
  };
}

/** Only the new settings endpoints are overridden; every other API stays closed. */
async function hierarchyFixture(page: Page, globalAdmin = true) {
  const base = await browserFixture(page, { administer: true, globalAdmin });
  let global = settings();
  let local: ChatSettingsSnapshot = {
    ...settings(),
    revision: 2,
    inheritance: inheritance(),
  };
  const writes: { path: string; body: ChatSettingsWrite }[] = [];
  let nextSave:
    | ((
        body: ChatSettingsWrite,
        current: ChatSettingsSnapshot,
      ) => ChatSettingsSnapshot | "conflict")
    | undefined;
  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (path !== globalSettings && path !== projectSettings)
      return route.fallback();
    const current = path === globalSettings ? global : local;
    if (request.method() === "GET") return route.fulfill({ json: current });
    expect(request.method()).toBe("PUT");
    expect(request.headers()["x-colossus-csrf"]).toBe("1");
    const body = request.postDataJSON() as ChatSettingsWrite;
    writes.push({ path, body });
    expect(body).not.toHaveProperty("credential_status");
    expect(body).not.toHaveProperty("telemetry");
    if (body.inheritance)
      expect(body.inheritance).not.toHaveProperty("provenance");
    const apply = nextSave;
    nextSave = undefined;
    const result = apply?.(body, current) ?? {
      ...current,
      ...clone(body),
      revision: current.revision + 1,
      ...(current.inheritance ? { inheritance: current.inheritance } : {}),
    };
    if (result === "conflict")
      return route.fulfill({ status: 409, json: { error: "conflict" } });
    expect(body.revision).toBe(current.revision);
    if (path === globalSettings) global = result;
    else local = result;
    return route.fulfill({ json: { revision: result.revision } });
  });
  return {
    ...base,
    writes,
    get global() {
      return global;
    },
    set global(value: ChatSettingsSnapshot) {
      global = value;
    },
    get local() {
      return local;
    },
    set local(value: ChatSettingsSnapshot) {
      local = value;
    },
    nextSave(callback: NonNullable<typeof nextSave>) {
      nextSave = callback;
    },
  };
}

async function choose(page: Page, name: string, option: string) {
  await page.getByRole("combobox", { name, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

async function publish(page: Page, instance = true) {
  await page
    .getByRole("button", {
      name: instance
        ? "Publish Control Plane defaults"
        : "Save project configuration",
      exact: true,
    })
    .click();
}

async function editVariable(page: Page, name: string, value: string) {
  await page.getByRole("button", { name: `Edit ${name}`, exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Variable name", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("textbox", { name: "Variable value", exact: true })
    .fill(value);
  await page
    .getByRole("button", { name: "Save variable", exact: true })
    .click();
}

test("global variables publish, project overrides reset and disabled inheritance can be restored", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/variables");
  await editVariable(page, "REGION", "global-west");
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]).toMatchObject({
    path: globalSettings,
    body: { revision: 5, variables: { REGION: "global-west" } },
  });
  await expect(
    page.getByRole("table", { name: "Configuration variables", exact: true }),
  ).toContainText("global-west");

  fixture.local = {
    ...fixture.local,
    variables: { REGION: "global-west" },
    inheritance: { ...inheritance(), global_revision: 6 },
  };
  await page.goto(`/projects/${project}/settings/variables`);
  const table = page.getByRole("table", {
    name: "Configuration variables",
    exact: true,
  });
  await expect(table).toContainText("Control Plane");
  fixture.nextSave((body, current) => ({
    ...current,
    ...body,
    revision: 3,
    inheritance: {
      ...current.inheritance!,
      overrides: { ...current.inheritance!.overrides, variables: ["REGION"] },
      provenance: {
        ...current.inheritance!.provenance,
        variables: { REGION: "project" },
      },
    },
  }));
  await editVariable(page, "REGION", "project-central");
  await expect(table).toContainText("project-central");
  expect(fixture.writes[1]?.body.inheritance).toEqual({ global_revision: 6 });
  const inherited = page.getByRole("region", {
    name: "Inherited configuration",
    exact: true,
  });
  await inherited.locator("summary").click();
  fixture.nextSave((body, current) => {
    expect(body.inheritance).toEqual({
      global_revision: 6,
      reset: { variables: ["REGION"] },
    });
    return {
      ...current,
      revision: 4,
      variables: { REGION: "global-west" },
      inheritance: { ...inheritance(), global_revision: 6 },
    };
  });
  await inherited
    .getByRole("button", { name: "Reset to Control Plane", exact: true })
    .click();
  await expect(table).toContainText("global-west");
  await expect(inherited).toContainText("Up to date");
  fixture.nextSave((body, current) => ({
    ...current,
    ...body,
    revision: 5,
    inheritance: {
      ...current.inheritance!,
      disabled: { ...current.inheritance!.disabled!, variables: ["REGION"] },
    },
  }));
  await table.getByRole("button", { name: "Remove", exact: true }).click();
  expect(fixture.writes).toHaveLength(3);
  await publish(page, false);
  await expect(inherited).toContainText("Disabled in this project");
  fixture.nextSave((body, current) => {
    expect(body.inheritance?.reset).toEqual({ variables: ["REGION"] });
    return {
      ...current,
      revision: 6,
      variables: { REGION: "global-west" },
      inheritance: { ...inheritance(), global_revision: 6 },
    };
  });
  await inherited
    .getByRole("button", { name: "Enable inherited definition", exact: true })
    .click();
  await expect(table).toContainText("global-west");
  expect(fixture.writes).toHaveLength(5);
  fixture.verify();
});

test("variable names and UTF-8 limits match the server contract before any publication", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  await page.goto("/admin/variables");
  await page.getByRole("button", { name: "Add variable", exact: true }).click();
  const name = page.getByRole("textbox", {
    name: "Variable name",
    exact: true,
  });
  const value = page.getByRole("textbox", {
    name: "Variable value",
    exact: true,
  });
  const save = page.getByRole("button", { name: "Save variable", exact: true });
  for (const invalid of ["with.dot", "with-dash", "with space", "é"]) {
    await name.fill(invalid);
    await value.fill("visible configuration");
    await save.click();
    await expect(page.getByRole("alert")).toContainText(
      "1–128 letters, digits, or underscores",
    );
    expect(fixture.writes).toHaveLength(0);
  }
  await name.fill("_REGION_2");
  await value.fill("contains\0nul");
  await save.click();
  await expect(page.getByRole("alert")).toContainText("4,096 UTF-8 bytes");
  expect(fixture.writes).toHaveLength(0);
  await value.fill("é".repeat(2049));
  await save.click();
  await expect(page.getByRole("alert")).toContainText("4,096 UTF-8 bytes");
  expect(fixture.writes).toHaveLength(0);
  await value.fill("é".repeat(2048));
  await save.click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]?.body.variables?.["_REGION_2"]).toHaveLength(2048);
  await expect(save).toBeHidden();
  fixture.verify();
});

test("global and project variables preserve an own __proto__ string key through publication and reread", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  for (const [path, endpoint] of [
    ["/admin/variables", globalSettings],
    [`/projects/${project}/settings/variables`, projectSettings],
  ] as const) {
    await page.goto(path);
    await page
      .getByRole("button", { name: "Add variable", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Variable name", exact: true })
      .fill("__proto__");
    await page
      .getByRole("textbox", { name: "Variable value", exact: true })
      .fill("visible own-key configuration");
    const before = fixture.writes.length;
    await page
      .getByRole("button", { name: "Save variable", exact: true })
      .click();
    await expect.poll(() => fixture.writes.length).toBe(before + 1);
    const submitted = fixture.writes[before]!;
    expect(submitted.path).toBe(endpoint);
    expect(Object.hasOwn(submitted.body.variables!, "__proto__")).toBe(true);
    expect(submitted.body.variables!["__proto__"]).toBe(
      "visible own-key configuration",
    );
    const row = page
      .getByRole("table", { name: "Configuration variables", exact: true })
      .getByRole("row")
      .filter({
        has: page.getByRole("button", { name: "Edit __proto__", exact: true }),
      });
    await expect(row).toContainText("visible own-key configuration");
    await page.reload();
    await expect(row).toContainText("visible own-key configuration");
  }
  fixture.verify();
});

test("a stale project revision is shown once and reconciled explicitly without retrying a mutation", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page, false);
  await page.goto(`/projects/${project}/settings/variables`);
  fixture.nextSave(() => "conflict");
  await editVariable(page, "REGION", "draft-kept");
  await expect(page.getByRole("alert")).toContainText(
    "Reload settings to reconcile",
  );
  expect(fixture.writes).toHaveLength(1);
  await expect(
    page.getByRole("textbox", { name: "Variable value", exact: true }),
  ).toHaveValue("draft-kept");
  fixture.local = {
    ...fixture.local,
    revision: 3,
    variables: { REGION: "another-admin" },
    inheritance: { ...inheritance(), global_revision: 7 },
  };
  await page
    .getByRole("button", { name: "Reload settings", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Variable value", exact: true }),
  ).toBeHidden();
  await expect(
    page.getByRole("table", { name: "Configuration variables", exact: true }),
  ).toContainText("another-admin");
  expect(fixture.writes).toHaveLength(1);
  fixture.verify([
    "Failed to load resource: the server responded with a status of 409 (Conflict)",
  ]);
});

test("legacy search credentials stay personal on edit and deletion clears labels and routing safely", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  fixture.global = {
    ...fixture.global,
    labels: { providers: {}, models: {}, search: { legacy: "Company search" } },
    search: {
      profiles: {
        legacy: {
          kind: "searxng",
          endpoint: "https://search.example",
          credentialReference: "legacy:credential",
        },
      },
      roles: { agent: "legacy" },
    },
    search_authentication: {},
  };
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/search");
  await page
    .getByRole("button", { name: "Edit Company search", exact: true })
    .click();
  await expect(
    page.getByRole("combobox", { name: "Search authentication", exact: true }),
  ).toContainText("Personal credential (each user)");
  await page
    .getByRole("textbox", { name: "Display label", exact: true })
    .fill("Renamed search");
  await page
    .getByRole("button", { name: "Save search service", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]?.body).toMatchObject({
    search_authentication: { legacy: "personal" },
    search: { profiles: { legacy: { credentialReference: "host:managed" } } },
    labels: { search: { legacy: "Renamed search" } },
  });
  await expect(
    page.getByRole("button", { name: "Edit Renamed search", exact: true }),
  ).toBeEnabled();
  // A routed service has no delete affordance until its route has been removed.
  await expect(
    page.getByRole("button", {
      name: "More actions for Renamed search",
      exact: true,
    }),
  ).toHaveCount(0);
  await choose(page, "Agent search route", "Not configured");
  await publish(page);
  await expect.poll(() => fixture.writes.length).toBe(2);
  await page
    .getByRole("button", {
      name: "More actions for Renamed search",
      exact: true,
    })
    .click();
  await page
    .getByRole("menuitem", { name: "Delete Renamed search", exact: true })
    .click();
  // Deletion is staged and reviewed through the same revisioned publication.
  expect(fixture.writes).toHaveLength(2);
  await publish(page);
  await expect.poll(() => fixture.writes.length).toBe(3);
  expect(fixture.writes[2]?.body.search).toEqual({ profiles: {}, roles: {} });
  expect(fixture.writes[2]?.body.search_authentication).toEqual({});
  expect(fixture.writes[2]?.body.labels?.search).toEqual({});
  await expect(
    page.getByRole("table", {
      name: "Configured search services",
      exact: true,
    }),
  ).toContainText("No search services yet");
  fixture.verify();
});

test("new shared search definitions and inherited search routes use explicit authentication and reset contracts", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  await page.goto("/admin/search");
  await page
    .getByRole("button", { name: "Add search service", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Display label", exact: true })
    .fill("Research search");
  await page
    .getByRole("textbox", { name: "Profile ID", exact: true })
    .fill("research");
  await page
    .getByRole("textbox", { name: /^Endpoint URL/u })
    .fill("https://research.example/search");
  await choose(
    page,
    "Search authentication",
    "Shared credential (explicit assignment)",
  );
  await page
    .getByRole("button", { name: "Save search service", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]?.body).toMatchObject({
    search_authentication: { research: "shared" },
    search: {
      profiles: {
        research: {
          kind: "searxng",
          credentialReference: "host:managed",
          endpoint: "https://research.example/search",
        },
      },
    },
  });
  const inherited = inheritance();
  inherited.global_entries!.roles = [
    "primary",
    "search:agent",
    "search:research",
  ];
  inherited.provenance.roles = {
    primary: "global",
    "search:agent": "project",
    "search:research": "global",
  };
  inherited.overrides.roles = ["search:agent"];
  fixture.local = {
    ...fixture.local,
    search: fixture.global.search,
    search_authentication: fixture.global.search_authentication,
    inheritance: inherited,
  };
  await page.goto(`/projects/${project}/settings/search`);
  const routes = page.getByRole("region", {
    name: "Inherited search routing",
    exact: true,
  });
  await routes.locator("summary").click();
  const table = routes.getByRole("table", {
    name: "Inherited definitions",
    exact: true,
  });
  await expect(table).toContainText("Agent search");
  await expect(table).toContainText("Research search");
  await expect(table).not.toContainText("primary");
  fixture.nextSave((body, current) => {
    expect(body.inheritance?.reset).toEqual({ roles: ["search:agent"] });
    return {
      ...current,
      revision: 3,
      inheritance: {
        ...inherited,
        overrides: { ...inherited.overrides, roles: [] },
      },
    };
  });
  await routes
    .getByRole("button", { name: "Reset to Control Plane", exact: true })
    .click();
  await expect(
    routes.getByRole("button", { name: "Reset to Control Plane", exact: true }),
  ).toHaveCount(0);
  fixture.verify();
});

test("unfinished Search drafts survive section changes and lock unrelated mutations until cancelled", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/search");
  await page
    .getByRole("button", { name: "Add search service", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Display label", exact: true })
    .fill("Unfinished research search");
  await page
    .getByRole("textbox", { name: "Profile ID", exact: true })
    .fill("unfinished");
  await page
    .getByRole("textbox", { name: /^Endpoint URL/u })
    .fill("https://draft-search.example");
  const administration = page
    .getByRole("navigation", { name: "Application", exact: true })
    .getByRole("region", { name: "Control Plane Admin", exact: true });
  await administration
    .getByRole("link", { name: "Providers", exact: true })
    .click();
  const notice = page
    .getByRole("status")
    .filter({ hasText: "An editor is still open in Search." });
  await expect(notice).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add provider", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", {
      name: "Publish Control Plane defaults",
      exact: true,
    }),
  ).toBeDisabled();
  await administration
    .getByRole("link", { name: "Search", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Display label", exact: true }),
  ).toHaveValue("Unfinished research search");
  await expect(
    page.getByRole("textbox", { name: /^Endpoint URL/u }),
  ).toHaveValue("https://draft-search.example");
  await expect(
    page.getByRole("button", { name: "Save search service", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await administration
    .getByRole("link", { name: "Providers", exact: true })
    .click();
  await expect(notice).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Add provider", exact: true }),
  ).toBeEnabled();
  expect(fixture.writes).toHaveLength(0);
  fixture.verify();
});

test("collector diagnostics are revision scoped and pending or console-only configurations cannot claim a collector test", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  const diagnostics: unknown[] = [];
  let mismatched = false;
  await page.route(
    "**/api/admin/hosted-settings/telemetry/test",
    async (route) => {
      expect(route.request().method()).toBe("POST");
      expect(route.request().headers()["x-colossus-csrf"]).toBe("1");
      diagnostics.push(route.request().postDataJSON());
      return route.fulfill({
        json: {
          status: "passed",
          message: mismatched
            ? "Stale delivery result"
            : "Diagnostic delivery acknowledged",
          revision: mismatched ? 5 : fixture.global.revision,
        },
      });
    },
  );
  await page.goto("/admin/telemetry");
  const testCollector = page.getByRole("button", {
    name: "Test collector",
    exact: true,
  });
  await testCollector.click();
  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "Diagnostic delivery acknowledged" }),
  ).toBeVisible();
  expect(diagnostics).toEqual([{ revision: 5 }]);
  fixture.global = { ...fixture.global, revision: 6 };
  await page
    .getByRole("button", { name: "Reload settings", exact: true })
    .click();
  await expect(
    page.getByText("Diagnostic delivery acknowledged", { exact: true }),
  ).toHaveCount(0);
  mismatched = true;
  await testCollector.click();
  await expect(testCollector).toBeEnabled();
  expect(diagnostics).toEqual([{ revision: 5 }, { revision: 6 }]);
  await expect(
    page.getByText("Stale delivery result", { exact: true }),
  ).toHaveCount(0);
  fixture.global = {
    ...fixture.global,
    revision: 7,
    telemetry: {
      state: "restart_required",
      configured_revision: 7,
      applied_revision: 5,
    },
  };
  await page
    .getByRole("button", { name: "Reload settings", exact: true })
    .click();
  await expect(testCollector).toBeDisabled();
  await expect(
    page.getByRole("region", { name: "OpenTelemetry settings", exact: true }),
  ).toContainText("Restart the Control Plane");
  fixture.global = {
    ...fixture.global,
    revision: 8,
    telemetry: { state: "active", configured_revision: 8, applied_revision: 8 },
    observability: {
      ...fixture.global.observability!,
      traces: { enabled: false, sampleRatio: 1 },
      logs: { ...fixture.global.observability!.logs, stdoutJson: true },
    },
  };
  await page
    .getByRole("button", { name: "Reload settings", exact: true })
    .click();
  await expect(testCollector).toBeDisabled();
  expect(diagnostics).toHaveLength(2);
  fixture.verify();
});

test("collector editor publishes typed safe signals and projects inherit a read-only collector", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/telemetry");
  await page
    .getByRole("button", { name: "Configure collector", exact: true })
    .click();
  const editor = page.getByRole("form", {
    name: "Edit Control Plane collector",
    exact: true,
  });
  await expect(
    editor.getByRole("textbox", { name: "Display label", exact: true }),
  ).toHaveCount(0);
  await editor
    .getByRole("textbox", { name: /^Service name/u })
    .fill("colossus-production");
  await editor
    .getByRole("textbox", { name: "Collector endpoint", exact: true })
    .fill("https://collector.example/v1/otlp");
  await choose(page, "Protocol", "OTLP HTTP/protobuf");
  await editor.getByRole("checkbox", { name: /^Metrics/u }).check();
  await editor
    .getByRole("spinbutton", { name: /^Trace sample \(millionths\)/u })
    .fill("250000");
  await editor
    .getByRole("textbox", { name: /^Attributes/u })
    .fill("environment=production\nenvironment=duplicate");
  await editor
    .getByRole("button", { name: "Save telemetry connection", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "Resource attribute names must be unique",
  );
  expect(fixture.writes).toHaveLength(0);
  await editor
    .getByRole("textbox", { name: /^Attributes/u })
    .fill("environment=production\nservice.namespace=colossus");
  await editor
    .getByRole("combobox", { name: "Audit content", exact: true })
    .click();
  await expect(
    page.getByRole("option", { name: "Full sensitive payloads", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("option", { name: "Metadata only", exact: true })
    .click();
  fixture.nextSave((body, current) => ({
    ...current,
    ...body,
    revision: 6,
    telemetry: {
      state: "restart_required",
      configured_revision: 6,
      applied_revision: 5,
    },
  }));
  await editor
    .getByRole("button", { name: "Save telemetry connection", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]?.body.observability).toMatchObject({
    serviceName: "colossus-production",
    resourceAttributes: {
      environment: "production",
      "service.namespace": "colossus",
    },
    traces: { enabled: true, sampleRatio: 0.25 },
    metrics: { enabled: true },
    logs: { journalPayloads: "metadata", acknowledgeSensitiveContent: false },
    otlp: {
      protocol: "http_protobuf",
      endpoint: "https://collector.example/v1/otlp",
    },
  });
  await expect(editor).toBeHidden();
  await expect(
    page.getByRole("button", { name: "Test collector", exact: true }),
  ).toBeDisabled();
  fixture.local = {
    ...fixture.local,
    observability: fixture.global.observability,
    telemetry: fixture.global.telemetry,
  };
  await page.goto(`/projects/${project}/settings/telemetry`);
  const inherited = page.getByRole("region", {
    name: "OpenTelemetry settings",
    exact: true,
  });
  await expect(inherited).toContainText("Inherited from Control Plane");
  await expect(inherited).toContainText("https://collector.example/v1/otlp");
  await expect(
    page.getByRole("button", { name: "Configure collector", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Test collector", exact: true }),
  ).toHaveCount(0);
  fixture.verify();
});

test("instance secrets remain write-only and assignments require an explicitly reviewed project and binding", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page);
  const destination = {
    id: "research",
    kind: "search" as const,
    label: "Shared research",
    endpoint: "https://search.example",
    credential_binding: "reviewed-binding-1",
    project_id: project,
    credential_source: "none" as const,
    credential_id: null as string | null,
  };
  let store: CredentialSnapshot = {
    revision: 0,
    credentials: [],
    connections: [destination],
  };
  const writes: {
    path: string;
    method: string;
    body: Record<string, unknown>;
  }[] = [];
  await page.route("**/api/admin/credential-store**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (request.method() === "GET")
      return route.fulfill({
        json: {
          ...store,
          connections:
            url.searchParams.get("project") === project
              ? store.connections
              : [],
        },
      });
    expect(request.headers()["x-colossus-csrf"]).toBe("1");
    const body = request.postDataJSON() as Record<string, unknown>;
    expect(body.revision).toBe(store.revision);
    writes.push({ path: url.pathname, method: request.method(), body });
    if (url.pathname === globalSecretStore) {
      expect(body).not.toHaveProperty("attachment");
      store = {
        ...store,
        credentials: [
          {
            id: "secret-1",
            label: String(body.label),
            kind: "api_key",
            legacy: false,
            used_by: [],
          },
        ],
      };
    } else if (url.pathname.includes("/attachments/")) {
      expect(body.credential_binding).toBe(destination.credential_binding);
      const attached = body.credential_id !== null;
      const connection = {
        ...destination,
        credential_id: attached ? "secret-1" : null,
      };
      store = {
        ...store,
        connections: [connection],
        credentials: store.credentials.map((row) => ({
          ...row,
          used_by: attached ? [connection] : [],
        })),
      };
    } else if (request.method() === "DELETE")
      store = { ...store, credentials: [] };
    store.revision += 1;
    return route.fulfill({ json: { revision: store.revision } });
  });
  await page.goto(`/admin/credentials?project=${project}`);
  await choose(
    page,
    "Project for credential assignments",
    "Secrets only — choose a project to assign",
  );
  await page
    .getByRole("button", { name: "Add credential", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Display label", exact: true })
    .fill("Instance research key");
  const secret = page.locator("input[aria-label='API key or token']");
  await secret.fill("synthetic-write-only-value");
  await page
    .getByRole("button", { name: "Save credential", exact: true })
    .click();
  await expect.poll(() => writes.length).toBe(1);
  await expect(secret).toHaveCount(0);
  await expect(
    page.getByRole("table", { name: "Configured credentials", exact: true }),
  ).toContainText("No assignments in selected project");
  await expect(
    page.getByRole("button", {
      name: "Attach existing credential",
      exact: true,
    }),
  ).toBeDisabled();
  await choose(
    page,
    "Project for credential assignments",
    "Browser test project",
  );
  await page
    .getByRole("button", { name: "Attach existing credential", exact: true })
    .click();
  await choose(page, "Saved credential", "Instance research key");
  await choose(page, "Credential connection", "Search: Shared research");
  await expect(
    page.getByText("Credential destination: https://search.example", {
      exact: true,
    }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Attach credential", exact: true })
    .click();
  await expect.poll(() => writes.length).toBe(2);
  expect(writes[1]).toEqual({
    path: `${globalSecretStore}/attachments/${project}/search/research`,
    method: "POST",
    body: {
      revision: 1,
      credential_id: "secret-1",
      credential_binding: "reviewed-binding-1",
    },
  });
  await page
    .getByRole("button", {
      name: "Details for Instance research key",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "Detach from Shared research", exact: true })
    .click();
  await expect.poll(() => writes.length).toBe(3);
  expect(writes[2]?.body.credential_id).toBeNull();
  await expect(page.locator("body")).not.toContainText(
    "synthetic-write-only-value",
  );
  await page
    .getByRole("button", {
      name: "More actions for Instance research key",
      exact: true,
    })
    .click();
  await page
    .getByRole("menuitem", {
      name: "Delete Instance research key",
      exact: true,
    })
    .click();
  await expect(
    page.getByText(
      "Removal disconnects this secret from every project where it is assigned.",
      { exact: false },
    ),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Remove credential", exact: true })
    .click();
  await expect.poll(() => writes.length).toBe(4);
  expect(writes[3]).toEqual({
    path: `${globalSecretStore}/secret-1`,
    method: "DELETE",
    body: { revision: 3 },
  });
  fixture.verify();
});

test("shared MCP credentials use named project Secrets with an explicit reviewed destination", async ({
  page,
}) => {
  const fixture = await hierarchyFixture(page, false);
  fixture.local = {
    ...fixture.local,
    mcp: {
      servers: {
        docs: {
          transport: "streamable_http",
          url: "https://tools.example/mcp",
          allowedTools: ["read"],
        },
      },
    },
    mcp_authentication: { docs: "shared" },
  };
  const destination = {
    id: "docs",
    kind: "mcp" as const,
    label: "Documentation tools",
    endpoint: "https://tools.example/mcp",
    credential_binding: "reviewed-project-mcp-binding",
    credential_id: null as string | null,
  };
  let store: CredentialSnapshot = {
    revision: 0,
    credentials: [],
    connections: [destination],
  };
  const writes: unknown[] = [];
  const path = `/api/projects/${project}/chat/credential-store/project`;
  await page.route(`**${path}`, async (route) => {
    const request = route.request();
    if (request.method() === "GET") return route.fulfill({ json: store });
    expect(request.method()).toBe("POST");
    expect(request.headers()["x-colossus-csrf"]).toBe("1");
    const body = request.postDataJSON();
    writes.push(body);
    expect(body).toEqual({
      revision: 0,
      label: "Shared documentation key",
      kind: "api_key",
      secret: "synthetic-project-mcp-secret",
      attachment: {
        kind: "mcp",
        connection: "docs",
        credential_binding: "reviewed-project-mcp-binding",
      },
    });
    const connection = { ...destination, credential_id: "project-mcp-key" };
    store = {
      revision: 1,
      connections: [connection],
      credentials: [
        {
          id: "project-mcp-key",
          label: body.label,
          kind: "api_key",
          legacy: false,
          used_by: [connection],
        },
      ],
    };
    return route.fulfill({ json: { revision: 1 } });
  });
  await page.goto(`/projects/${project}/settings/mcp`);
  await expect(
    page.locator("input[aria-label='API key or token']"),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Manage shared credential", exact: true })
    .click();
  const secret = page.locator("input[aria-label='API key or token']");
  await expect(secret).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Credential connection", exact: true }),
  ).toContainText("MCP: Documentation tools");
  await expect(
    page.getByRole("form", { name: "API key or token", exact: true }),
  ).toContainText("Credential destination: https://tools.example/mcp");
  await page
    .getByRole("textbox", { name: "Display label", exact: true })
    .fill("Shared documentation key");
  await page.screenshot({
    path: "output/playwright/settings-hierarchy-mcp-named-project-secret.png",
  });
  await page
    .getByRole("button", { name: "Save credential", exact: true })
    .scrollIntoViewIfNeeded();
  await page.screenshot({
    path: "output/playwright/settings-hierarchy-mcp-named-project-secret-footer.png",
  });
  await secret.fill("synthetic-project-mcp-secret");
  await page
    .getByRole("button", { name: "Save credential", exact: true })
    .click();
  await expect.poll(() => writes.length).toBe(1);
  await expect(secret).toHaveCount(0);
  await expect(
    page.getByRole("table", { name: "Configured credentials", exact: true }),
  ).toContainText("Shared documentation key");
  await expect(page.locator("body")).not.toContainText(
    "synthetic-project-mcp-secret",
  );
  expect(fixture.writes).toHaveLength(0);
  fixture.verify();
});

test("personal variable edits and removal stay within the selected user's project endpoint", async ({
  page,
}) => {
  const fixture = await browserFixture(page);
  let saved = {
    revision: 2,
    variables: { REGION: "personal-west" } as Record<string, string>,
  };
  const writes: (typeof saved)[] = [];
  await page.route(
    `**/api/projects/${project}/chat/variables/personal`,
    async (route) => {
      const request = route.request();
      if (request.method() === "GET") return route.fulfill({ json: saved });
      expect(request.method()).toBe("PUT");
      expect(request.headers()["x-colossus-csrf"]).toBe("1");
      const body = request.postDataJSON() as typeof saved;
      expect(body.revision).toBe(saved.revision);
      writes.push(body);
      saved = { revision: saved.revision + 1, variables: body.variables };
      return route.fulfill({ json: { revision: saved.revision } });
    },
  );
  await page.goto(`/settings/variables?project=${project}`);
  await editVariable(page, "REGION", "personal-central");
  const table = page.getByRole("table", {
    name: "Your project variables",
    exact: true,
  });
  await expect(table).toContainText("personal-central");
  expect(writes).toEqual([
    { revision: 2, variables: { REGION: "personal-central" } },
  ]);
  await table.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(table).toContainText("No personal variables configured.");
  expect(writes[1]).toEqual({ revision: 3, variables: {} });
  expect(fixture.settingsSubmissions).toHaveLength(0);
  fixture.verify();
});

test("personal variables use their own revisioned endpoint and readers cannot edit them", async ({
  page,
}) => {
  const fixture = await browserFixture(page, { permissions: ["read"] });
  const writes: unknown[] = [];
  await page.route(
    `**/api/projects/${project}/chat/variables/personal`,
    async (route) => {
      if (route.request().method() === "GET")
        return route.fulfill({
          json: { revision: 2, variables: { REGION: "personal-west" } },
        });
      writes.push(route.request().postDataJSON());
      return route.fulfill({ json: { revision: 3 } });
    },
  );
  await page.goto(`/settings/variables?project=${project}`);
  const variables = page.getByRole("region", {
    name: "Personal project variables",
    exact: true,
  });
  await expect(variables).toContainText("personal-west");
  await expect(
    variables.getByRole("button", { name: "Add variable", exact: true }),
  ).toBeDisabled();
  await expect(
    variables.getByRole("button", { name: "Edit REGION", exact: true }),
  ).toBeDisabled();
  await expect(
    variables.getByRole("button", { name: "Remove", exact: true }),
  ).toBeDisabled();
  expect(writes).toHaveLength(0);
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`new settings editors fit Large desktop and compact ${appearance.label} layouts`, async ({
    page,
  }) => {
    const fixture = await hierarchyFixture(page);
    await selectAppearance(page, appearance);
    await page.goto("/admin/telemetry");
    await page
      .getByRole("button", { name: "Configure collector", exact: true })
      .click();
    for (const width of [1440, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      const editor = page.getByRole("form", {
        name: "Edit Control Plane collector",
        exact: true,
      });
      await editor
        .getByRole("textbox", { name: /^Service name/u })
        .scrollIntoViewIfNeeded();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      const bounds = await editor.evaluate((element) => ({
        rect: element.getBoundingClientRect().toJSON(),
        viewport: innerWidth,
      }));
      expect(bounds.rect.left).toBeGreaterThanOrEqual(0);
      expect(bounds.rect.right).toBeLessThanOrEqual(bounds.viewport);
      await page.screenshot({
        path: `output/playwright/settings-hierarchy-${appearance.id}-large-${width}.png`,
        fullPage: true,
      });
      await editor
        .getByRole("button", { name: "Save telemetry connection", exact: true })
        .scrollIntoViewIfNeeded();
      await expect(
        editor.getByRole("button", {
          name: "Save telemetry connection",
          exact: true,
        }),
      ).toBeInViewport();
      await page.screenshot({
        path: `output/playwright/settings-hierarchy-${appearance.id}-large-${width}-footer.png`,
        fullPage: true,
      });
    }
    fixture.verify();
  });
}
