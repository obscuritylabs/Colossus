import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Button,
  CatalogInventory,
  DropdownSelect,
  TextInput,
} from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import {
  SearchSettingsEditor,
  type SearchSettingsDraft,
} from "@colossus/ui/components/SearchSettingsEditor";
import {
  TelemetrySettingsEditor,
  type TelemetrySettingsDraft,
} from "@colossus/ui/components/TelemetrySettingsEditor";
import type { CredentialTarget } from "./credential-types";
import {
  SettingsEditorPage,
  SettingsEditorUnavailable,
  type SettingsEditorNavigation,
} from "./settings-editor";
import { request } from "./api";
import { errorMessage } from "./resources";
import {
  validProfile,
  validVariableName,
  type Authentication,
  type ChatSettingsSnapshot,
  type ChatSettingsWrite,
  type ObservabilitySettings,
  type SearchProfile,
} from "./chat-settings";

function effectiveSearchAuthentication(
  draft: ChatSettingsWrite,
  id: string,
  profile?: SearchProfile,
): Authentication {
  return (
    draft.search_authentication?.[id] ??
    (profile?.credentialReference ? "personal" : "none")
  );
}

const telemetryDefaults: ObservabilitySettings = {
  enabled: false,
  serviceName: "colossus",
  resourceAttributes: {},
  traces: { enabled: false, sampleRatio: 1 },
  metrics: { enabled: false, exportIntervalMs: 60000 },
  logs: {
    otlp: false,
    stdoutJson: false,
    journalPayloads: "disabled",
    acknowledgeSensitiveContent: false,
  },
  otlp: {
    endpoint: null,
    protocol: "grpc",
    timeoutMs: 10000,
    acknowledgeInsecureTransport: false,
  },
};
function telemetryDraft(value: ObservabilitySettings): TelemetrySettingsDraft {
  return {
    resourceId: "control-plane",
    label: "Control Plane collector",
    name: value.serviceName,
    endpoint: value.otlp.endpoint,
    protocol: value.otlp.protocol,
    timeoutMs: value.otlp.timeoutMs,
    tracesEnabled: value.traces.enabled,
    traceSampleRatioMillionths: Math.round(value.traces.sampleRatio * 1000000),
    metricsEnabled: value.metrics.enabled,
    metricExportIntervalMs: value.metrics.exportIntervalMs,
    logsOtlp: value.logs.otlp,
    logsStdoutJson: value.logs.stdoutJson,
    journalPayloads: value.logs.journalPayloads,
    acknowledgeSensitiveContent: false,
    acknowledgeInsecureTransport: value.otlp.acknowledgeInsecureTransport,
    resourceAttributesText: Object.entries(value.resourceAttributes)
      .map(([key, text]) => `${key}=${text}`)
      .join("\n"),
  };
}

/** The Web host binds shared editors to an authenticated, revisioned settings draft. */
export function ConnectionSettingsExtras({
  section,
  draft,
  saved,
  instanceScope,
  busy,
  navigationBusy = busy,
  onChange,
  onSave,
  onCredentials,
  onPersonalCredentials,
  onEditingChange,
  resetEditor = 0,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
}: SettingsEditorNavigation & {
  section: "search" | "variables" | "telemetry";
  draft: ChatSettingsWrite;
  saved: ChatSettingsSnapshot;
  instanceScope: boolean;
  busy: boolean;
  navigationBusy?: boolean;
  onChange: (value: ChatSettingsWrite) => void;
  onSave: (value: ChatSettingsWrite) => void;
  onCredentials?: ((target?: CredentialTarget) => void) | undefined;
  onPersonalCredentials?: ((target?: CredentialTarget) => void) | undefined;
  onEditingChange?: (
    section: "search" | "variables" | "telemetry" | null,
  ) => void;
  resetEditor?: number;
}) {
  const [searchEditor, setSearchEditor] = useState<
    (SearchSettingsDraft & { authentication: Authentication }) | null
  >(null);
  const [variable, setVariable] = useState<{
    original: string;
    name: string;
    value: string;
  } | null>(null);
  const [telemetry, setTelemetry] = useState<TelemetrySettingsDraft | null>(
    null,
  );
  const openingValue = useRef("");
  const formValue = JSON.stringify(searchEditor ?? variable ?? telemetry);
  useEffect(() => {
    onDirtyChange?.(formValue !== "null" && formValue !== openingValue.current);
  }, [formValue, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const [error, setError] = useState("");
  const [testing, setTesting] = useState(false);
  const [diagnostic, setDiagnostic] = useState("");
  const alive = useRef(true);
  const currentRevision = useRef(saved.revision);
  currentRevision.current = saved.revision;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    setSearchEditor(null);
    setVariable(null);
    setTelemetry(null);
    setError("");
    setDiagnostic("");
  }, [saved.revision]);
  useEffect(() => {
    setSearchEditor(null);
    setVariable(null);
    setTelemetry(null);
    setError("");
  }, [resetEditor]);
  useEffect(() => {
    onEditingChange?.(
      searchEditor
        ? "search"
        : variable
          ? "variables"
          : telemetry
            ? "telemetry"
            : null,
    );
  }, [
    Boolean(searchEditor),
    Boolean(variable),
    Boolean(telemetry),
    onEditingChange,
  ]);
  const search = draft.search ?? { profiles: {}, roles: {} };
  const attributes = draft.variables ?? {};
  const routeKey = JSON.stringify([
    section,
    editorLocation,
    saved.revision,
    resetEditor,
  ]);
  useEffect(() => {
    if (!editorLocation) return;
    setSearchEditor(null);
    setVariable(null);
    setTelemetry(null);
    setError("");
    if (
      section === "search" &&
      (editorLocation.action === "new" ||
        (editorLocation.id &&
          Object.hasOwn(search.profiles, editorLocation.id)))
    )
      editSearch(editorLocation.action === "edit" ? editorLocation.id : "");
    else if (
      section === "variables" &&
      (editorLocation.action === "new" ||
        (editorLocation.id && Object.hasOwn(attributes, editorLocation.id)))
    ) {
      const name = editorLocation.action === "edit" ? editorLocation.id! : "";
      const value = { original: name, name, value: attributes[name] ?? "" };
      openingValue.current = JSON.stringify(value);
      setVariable(value);
    } else if (
      section === "telemetry" &&
      instanceScope &&
      editorLocation.action === "edit" &&
      !editorLocation.id
    )
      editTelemetry();
  }, [routeKey]);
  function closeEditor() {
    if (editorLocation && onCloseEditor) {
      onCloseEditor();
      return;
    }
    setSearchEditor(null);
    setVariable(null);
    setTelemetry(null);
    setError("");
    onCloseEditor?.();
  }
  function openVariable(name = "") {
    if (onOpenEditor)
      onOpenEditor(name ? { action: "edit", id: name } : { action: "new" });
    else {
      const value = { original: name, name, value: attributes[name] ?? "" };
      openingValue.current = JSON.stringify(value);
      setVariable(value);
    }
  }
  function editTelemetry() {
    const value = telemetryDraft(draft.observability ?? telemetryDefaults);
    openingValue.current = JSON.stringify(value);
    setTelemetry(value);
  }
  const page = (title: string, children: ReactNode) => (
    <SettingsEditorPage
      title={title}
      backLabel={
        section === "search"
          ? "Search"
          : section === "variables"
            ? "Variables"
            : "Telemetry"
      }
      busy={navigationBusy}
      showHeading={false}
      onBack={closeEditor}
      description={
        instanceScope
          ? "Control Plane defaults flow into projects. Shared credentials require a separate explicit assignment."
          : "Changes apply to this project. Editing an inherited definition creates a project override."
      }
    >
      {error ? <p role="alert">{error}</p> : null}
      {children}
    </SettingsEditorPage>
  );
  if (
    editorLocation &&
    ((editorLocation.action === "edit" &&
      editorLocation.id &&
      !(section === "search"
        ? Object.hasOwn(search.profiles, editorLocation.id)
        : section === "variables"
          ? Object.hasOwn(attributes, editorLocation.id)
          : false)) ||
      (section === "variables" &&
        editorLocation.action === "edit" &&
        !editorLocation.id) ||
      (section === "telemetry" &&
        (!instanceScope ||
          editorLocation.action !== "edit" ||
          editorLocation.id)))
  )
    return (
      <SettingsEditorUnavailable
        backLabel={
          section === "search"
            ? "Search"
            : section === "variables"
              ? "Variables"
              : "Telemetry"
        }
        onBack={closeEditor}
      />
    );
  const columns: DataTableColumn<{ name: string; value: string }>[] = [
    { id: "name", label: "Name", value: (row) => row.name },
    { id: "value", label: "Value", value: (row) => row.value },
    {
      id: "source",
      label: "Source",
      value: (row) =>
        saved.inheritance?.provenance.variables[row.name] === "global"
          ? "Control Plane"
          : instanceScope
            ? "Control Plane"
            : "Project",
    },
    {
      id: "actions",
      label: "Actions",
      value: () => "",
      cell: (row) => (
        <div className="chat-settings-row-actions">
          <Button
            disabled={busy || Boolean(variable)}
            onClick={() => openVariable(row.name)}
          >
            Edit {row.name}
          </Button>
          <Button
            disabled={busy || Boolean(variable)}
            variant="danger"
            onClick={() => {
              const variables = { ...attributes };
              delete variables[row.name];
              onChange({ ...draft, variables });
            }}
          >
            Remove
          </Button>
        </div>
      ),
    },
  ];
  const variableForm = variable ? (
    <form
      className="chat-settings-editor"
      onSubmit={(event) => {
        event.preventDefault();
        if (busy) return;
        if (
          !validVariableName(variable.name) ||
          new TextEncoder().encode(variable.value).length > 4096 ||
          variable.value.includes("\0")
        ) {
          setError(
            "Use 1–128 letters, digits, or underscores for the name and at most 4,096 UTF-8 bytes for the value.",
          );
          return;
        }
        if (!variable.original && Object.hasOwn(attributes, variable.name)) {
          setError(
            "That variable already exists. Edit it to change its value.",
          );
          return;
        }
        const variables = { ...attributes };
        if (variable.original !== variable.name)
          delete variables[variable.original];
        onSave({
          ...draft,
          variables: { ...variables, [variable.name]: variable.value },
        });
      }}
    >
      <h3>{variable.original ? "Edit variable" : "Add variable"}</h3>
      <label>
        Name
        <TextInput
          aria-label="Variable name"
          value={variable.name}
          maxLength={128}
          disabled={busy || Boolean(variable.original)}
          onChange={(event) =>
            setVariable({ ...variable, name: event.target.value })
          }
          required
        />
      </label>
      <label>
        Value
        <TextInput
          aria-label="Variable value"
          value={variable.value}
          maxLength={4096}
          disabled={busy}
          onChange={(event) =>
            setVariable({ ...variable, value: event.target.value })
          }
        />
      </label>
      <p className="field-help">
        For API keys and tokens, use Secrets. Variable values are visible
        configuration.
      </p>
      <div className="chat-settings-actions">
        <Button type="submit" variant="primary" disabled={busy}>
          Save variable
        </Button>
        <Button disabled={busy} onClick={closeEditor}>
          Cancel
        </Button>
      </div>
    </form>
  ) : null;
  if (section === "variables" && editorLocation)
    return page(
      editorLocation.action === "new" ? "Add variable" : "Edit variable",
      variableForm,
    );
  if (section === "variables")
    return (
      <section aria-label="Configuration variables">
        <header className="catalog-heading">
          <div>
            <h2>Variables</h2>
            <p>
              Reusable, nonsecret configuration values. Global values flow into
              projects; project values can override them. Values are available
              to authorized project members and are not injected into process
              environments.
            </p>
          </div>
          <Button
            disabled={busy || Boolean(variable)}
            onClick={() => openVariable()}
          >
            Add variable
          </Button>
        </header>
        {error ? <p role="alert">{error}</p> : null}
        {variableForm}
        <DataTable
          data={Object.entries(attributes).map(([name, value]) => ({
            name,
            value,
          }))}
          columns={columns}
          getRowId={(row) => row.name}
          label="Configuration variables"
          search={{ columnId: "name", label: "Search variables" }}
          empty="No variables configured."
        />
      </section>
    );
  if (section === "telemetry") {
    const value = draft.observability ?? telemetryDefaults;
    const telemetryForm =
      instanceScope && telemetry ? (
        <TelemetrySettingsEditor
          showDisplayLabel={false}
          draft={telemetry}
          busy={busy}
          onChange={setTelemetry}
          onCancel={closeEditor}
          allowSensitivePayloads={false}
          notice={
            <p className="sync-notice">
              Collector changes apply after restarting the Control Plane.
              Private conversation and tool payloads are excluded.
            </p>
          }
          onSave={() => {
            try {
              if (telemetry.endpoint) {
                const endpoint = new URL(telemetry.endpoint);
                if (
                  !["http:", "https:"].includes(endpoint.protocol) ||
                  endpoint.username ||
                  endpoint.password ||
                  endpoint.search ||
                  endpoint.hash
                )
                  throw new Error(
                    "Use an HTTP or HTTPS collector URL without embedded credentials, a query, or a fragment.",
                  );
              }
              if (!telemetry.name.trim())
                throw new Error("Enter a service name.");
              const resourceAttributes: Record<string, string> =
                Object.create(null);
              for (const line of telemetry.resourceAttributesText
                .split("\n")
                .filter((line) => line.trim())) {
                const split = line.indexOf("=");
                if (split < 1)
                  throw new Error(
                    "Enter resource attributes as one name=value pair per line.",
                  );
                const key = line.slice(0, split).trim();
                if (Object.hasOwn(resourceAttributes, key))
                  throw new Error("Resource attribute names must be unique.");
                resourceAttributes[key] = line.slice(split + 1).trim();
              }
              onSave({
                ...draft,
                observability: {
                  enabled:
                    telemetry.tracesEnabled ||
                    telemetry.metricsEnabled ||
                    telemetry.logsOtlp ||
                    telemetry.logsStdoutJson,
                  serviceName: telemetry.name.trim(),
                  resourceAttributes,
                  traces: {
                    enabled: telemetry.tracesEnabled,
                    sampleRatio: telemetry.traceSampleRatioMillionths / 1000000,
                  },
                  metrics: {
                    enabled: telemetry.metricsEnabled,
                    exportIntervalMs: telemetry.metricExportIntervalMs,
                  },
                  logs: {
                    otlp: telemetry.logsOtlp,
                    stdoutJson: telemetry.logsStdoutJson,
                    journalPayloads: telemetry.journalPayloads,
                    acknowledgeSensitiveContent: false,
                  },
                  otlp: {
                    endpoint: telemetry.endpoint,
                    protocol: telemetry.protocol,
                    timeoutMs: telemetry.timeoutMs,
                    acknowledgeInsecureTransport:
                      telemetry.acknowledgeInsecureTransport,
                  },
                },
              });
            } catch (cause) {
              setError(
                cause instanceof Error
                  ? cause.message
                  : "Review the collector configuration.",
              );
            }
          }}
        />
      ) : null;
    if (editorLocation) return page("Configure collector", telemetryForm);
    return (
      <section aria-label="OpenTelemetry settings">
        <header className="catalog-heading">
          <div>
            <h2>OpenTelemetry</h2>
            <p>
              One Control Plane collector is inherited by every project.
              Execution spans and lifecycle logs carry project identity; metrics
              remain instance aggregates.
            </p>
          </div>
          {instanceScope ? (
            <Button
              disabled={busy || Boolean(telemetry)}
              onClick={() =>
                onOpenEditor
                  ? onOpenEditor({ action: "edit" })
                  : editTelemetry()
              }
            >
              Configure collector
            </Button>
          ) : null}
        </header>
        <p role="status" className="sync-notice">
          {saved.telemetry?.state === "restart_required"
            ? "Collector changes are saved. Restart the Control Plane to apply them; the running exporter is unchanged."
            : saved.telemetry?.state === "active"
              ? "The exporter is initialized with the saved configuration. Use Test collector to check diagnostic delivery."
              : "Telemetry export is disabled."}
        </p>
        {instanceScope ? (
          <>
            <Button
              disabled={
                busy ||
                testing ||
                saved.telemetry?.state !== "active" ||
                !value.enabled ||
                !(
                  value.traces.enabled ||
                  value.metrics.enabled ||
                  value.logs.otlp
                )
              }
              onClick={async () => {
                const revision = saved.revision;
                setTesting(true);
                setDiagnostic("");
                try {
                  const result = await request<{
                    status: "passed" | "failed";
                    message: string;
                    revision: number;
                  }>("/api/admin/hosted-settings/telemetry/test", { revision });
                  if (
                    alive.current &&
                    currentRevision.current === revision &&
                    result.revision === revision
                  )
                    setDiagnostic(result.message);
                } catch (cause) {
                  if (alive.current && currentRevision.current === revision)
                    setDiagnostic(errorMessage(cause));
                } finally {
                  if (alive.current) setTesting(false);
                }
              }}
            >
              {testing ? "Testing collector…" : "Test collector"}
            </Button>
            {diagnostic ? <p role="status">{diagnostic}</p> : null}
          </>
        ) : null}
        <dl className="stat-list">
          <div>
            <dt>Configuration</dt>
            <dd>
              {instanceScope
                ? "Control Plane administrator"
                : "Inherited from Control Plane"}
            </dd>
          </div>
          <div>
            <dt>Collector</dt>
            <dd>{value.otlp.endpoint || "Not configured"}</dd>
          </div>
          <div>
            <dt>Service</dt>
            <dd>{value.serviceName}</dd>
          </div>
          <div>
            <dt>Signals</dt>
            <dd>
              {[
                value.traces.enabled && "Traces",
                value.metrics.enabled && "Metrics",
                value.logs.otlp && "Lifecycle logs",
              ]
                .filter(Boolean)
                .join(" · ") || "None"}
            </dd>
          </div>
        </dl>
        <p className="field-help">
          Private conversation and tool payloads are excluded. Collector changes
          take effect after a Control Plane restart.
        </p>
        {error ? <p role="alert">{error}</p> : null}
        {telemetryForm}
      </section>
    );
  }
  function openSearch(id = "") {
    if (onOpenEditor)
      onOpenEditor(id ? { action: "edit", id } : { action: "new" });
    else editSearch(id);
  }
  function editSearch(id = "") {
    const profile = search.profiles[id];
    setError("");
    const value = {
      resourceId: id || null,
      label: draft.labels?.search?.[id] ?? id,
      profile: id,
      kind: profile?.kind ?? "searxng",
      endpoint: profile?.endpoint ?? "",
      authHeader: profile?.authHeader ?? "",
      timeoutMs: profile?.timeoutMs ?? 30000,
      userAgent: profile?.userAgent ?? "colossus",
      authentication: effectiveSearchAuthentication(draft, id, profile),
    };
    openingValue.current = JSON.stringify(value);
    setSearchEditor(value);
  }
  const searchForm = searchEditor ? (
    <SearchSettingsEditor
      draft={searchEditor}
      busy={busy}
      onChange={(value) =>
        setSearchEditor({
          ...value,
          authentication:
            value.kind === "serp_api" && value.authentication === "none"
              ? "personal"
              : value.authentication,
        })
      }
      onCancel={closeEditor}
      profileReadOnly={Boolean(searchEditor.resourceId)}
      authentication={
        <label>
          Authentication
          <DropdownSelect
            disabled={busy}
            aria-label="Search authentication"
            value={searchEditor.authentication}
            onChange={(event) =>
              setSearchEditor({
                ...searchEditor,
                authentication: event.target.value as Authentication,
              })
            }
          >
            <option value="none">No authentication</option>
            <option value="personal">Personal credential (each user)</option>
            <option value="shared">
              Shared credential (explicit assignment)
            </option>
          </DropdownSelect>
        </label>
      }
      onSave={() => {
        if (
          !searchEditor.label.trim() ||
          !validProfile(searchEditor.profile) ||
          (!searchEditor.resourceId &&
            Object.hasOwn(search.profiles, searchEditor.profile))
        ) {
          setError(
            "Enter a display label and a unique, valid search profile ID.",
          );
          return;
        }
        if (
          searchEditor.kind === "serp_api" &&
          searchEditor.authentication === "none"
        ) {
          setError(
            "SerpAPI requires a personal or explicitly shared credential.",
          );
          return;
        }
        try {
          const endpoint = new URL(searchEditor.endpoint.trim());
          if (
            !["http:", "https:"].includes(endpoint.protocol) ||
            endpoint.username ||
            endpoint.password ||
            endpoint.hash
          )
            throw new Error();
        } catch {
          setError(
            "Enter an HTTP or HTTPS search endpoint without embedded credentials or a fragment.",
          );
          return;
        }
        if (
          !Number.isSafeInteger(searchEditor.timeoutMs) ||
          searchEditor.timeoutMs < 1 ||
          searchEditor.timeoutMs > 300000
        ) {
          setError("Use a timeout between 1 and 300,000 milliseconds.");
          return;
        }
        const profile = {
          ...(search.profiles[searchEditor.profile]?.kind === searchEditor.kind
            ? search.profiles[searchEditor.profile]
            : {}),
          kind: searchEditor.kind,
          endpoint: searchEditor.endpoint.trim(),
          timeoutMs: searchEditor.timeoutMs,
          credentialReference:
            searchEditor.authentication === "none" ? null : "host:managed",
          ...(searchEditor.kind === "searxng"
            ? {
                authHeader: searchEditor.authHeader || "X-Searxng-Key",
                userAgent: searchEditor.userAgent || "colossus",
              }
            : {}),
        };
        onSave({
          ...draft,
          labels: {
            ...(draft.labels ?? { providers: {}, models: {} }),
            search: {
              ...draft.labels?.search,
              [searchEditor.profile]: searchEditor.label.trim(),
            },
          },
          search: {
            ...search,
            profiles: {
              ...search.profiles,
              [searchEditor.profile]: profile,
            },
          },
          search_authentication: {
            ...draft.search_authentication,
            [searchEditor.profile]: searchEditor.authentication,
          },
        });
      }}
    />
  ) : null;
  const searchRouting = (
    <section className="control-card">
      <h3>Search routing</h3>
      {["agent", "research"].map((role) => (
        <label key={role}>
          {role === "agent" ? "Agent search" : "Research search"}
          <DropdownSelect
            aria-label={`${role === "agent" ? "Agent" : "Research"} search route`}
            value={search.roles[role] ?? ""}
            disabled={
              busy ||
              Boolean(searchEditor) ||
              Boolean(onOpenEditor && !editorLocation)
            }
            onChange={(event) => {
              const roles = { ...search.roles };
              if (event.target.value) roles[role] = event.target.value;
              else delete roles[role];
              onChange({ ...draft, search: { ...search, roles } });
            }}
          >
            <option value="">Not configured</option>
            {Object.keys(search.profiles).map((id) => (
              <option key={id} value={id}>
                {id}
              </option>
            ))}
          </DropdownSelect>
        </label>
      ))}
    </section>
  );
  if (editorLocation)
    return page(
      editorLocation.action === "edit" && !editorLocation.id
        ? "Search routing"
        : editorLocation.action === "new"
          ? "Add search service"
          : "Edit search service",
      editorLocation.action === "edit" && !editorLocation.id ? (
        <>
          {searchRouting}
          <div className="chat-settings-actions">
            <Button disabled={busy} onClick={closeEditor}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={
                busy ||
                JSON.stringify(draft.search) === JSON.stringify(saved.search)
              }
              onClick={() => onSave(draft)}
            >
              Save search routing
            </Button>
          </div>
        </>
      ) : (
        searchForm
      ),
    );
  return (
    <section aria-label="Search services">
      <CatalogInventory
        kind="search"
        description={
          instanceScope
            ? "Publish search definitions for projects to inherit, then choose default routes."
            : "Connect a search service and choose where this project's agent and research searches run."
        }
        usageLabel="Search routes"
        headingLevel={2}
        summary={`${Object.keys(search.profiles).length} search services`}
        busy={busy}
        editing={Boolean(searchEditor)}
        onAdd={() => openSearch()}
        emptyDescription="Connect a search service, then choose where agent and research searches run."
        rows={Object.entries(search.profiles).map(([id, value]) => ({
          id,
          label: draft.labels?.search?.[id] ?? id,
          name: draft.labels?.search?.[id] ?? id,
          description: value.endpoint ?? "SerpAPI",
          searchText: `${id} ${draft.labels?.search?.[id] ?? ""} ${value.endpoint ?? ""}`,
          connection:
            effectiveSearchAuthentication(draft, id, value) === "none"
              ? "No authentication"
              : "Explicit credential",
          usage:
            Object.entries(search.roles)
              .filter(([, profile]) => profile === id)
              .map(([role]) => role)
              .join(", ") || "Not routed",
          onEdit: () => openSearch(id),
          ...(!Object.values(search.roles).includes(id)
            ? {
                onDelete: () => {
                  const profiles = { ...search.profiles };
                  delete profiles[id];
                  const search_authentication = {
                    ...draft.search_authentication,
                  };
                  delete search_authentication[id];
                  const labels = {
                    ...(draft.labels ?? { providers: {}, models: {} }),
                    search: { ...draft.labels?.search },
                  };
                  delete labels.search[id];
                  onChange({
                    ...draft,
                    search: { ...search, profiles },
                    search_authentication,
                    labels,
                  });
                },
              }
            : {}),
          details: (
            <>
              <p>
                {saved.inheritance?.provenance.search[id] === "global"
                  ? "Inherited from Control Plane"
                  : instanceScope
                    ? "Control Plane definition"
                    : "Project definition"}
              </p>
              {(effectiveSearchAuthentication(draft, id, value) === "personal"
                ? onPersonalCredentials
                : onCredentials) &&
              effectiveSearchAuthentication(draft, id, value) !== "none" ? (
                <Button
                  disabled={busy || Boolean(searchEditor)}
                  onClick={() =>
                    (effectiveSearchAuthentication(draft, id, value) ===
                      "personal"
                      ? onPersonalCredentials
                      : onCredentials)?.({ kind: "search", id })
                  }
                >
                  Manage search credential
                </Button>
              ) : null}
            </>
          ),
        }))}
      >
        {error ? <p role="alert">{error}</p> : null}
        {searchForm}
      </CatalogInventory>
      {onOpenEditor ? (
        <section className="control-card">
          <header className="catalog-heading">
            <div>
              <h3>Search routing</h3>
              <p>
                Agent: {search.roles.agent ?? "Not configured"} · Research:{" "}
                {search.roles.research ?? "Not configured"}
              </p>
            </div>
            <Button
              disabled={busy || Boolean(searchEditor)}
              onClick={() => onOpenEditor({ action: "edit" })}
            >
              Configure search routing
            </Button>
          </header>
        </section>
      ) : (
        searchRouting
      )}
    </section>
  );
}
