import { CatalogInventory } from "@colossus/ui";
import { ProviderIcon } from "@colossus/ui/components/ProviderIcon";
import { ModelRoleRouting } from "@colossus/ui/components/ModelRoleRouting";
import { ModelRoutingOverview } from "@colossus/ui/components/ModelRoutingOverview";
import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { projectPath, request } from "./api";
import { LoadState } from "./Home";
import { errorMessage, useResource } from "./resources";
import { ChatSettingsEditor, type SettingsEditor } from "./ChatSettingsEditors";
import {
  settingsWrite,
  type Authentication,
  type ChatSettingsSnapshot,
  type ChatSettingsWrite,
} from "./chat-settings";

interface InventoryRow {
  id: string;
  detail: string;
  authentication?: Authentication;
}

import { ChatSharedCredential } from "./ChatSharedCredential";
import { ChatProjectCredentials } from "./ChatProjectCredentials";
import type { CredentialSetup, CredentialTarget } from "./credential-types";

export function ChatSettings({
  project,
  identity,
  canAdminister,
  onSaved,
  section = "providers",
  onCredentials,
  onPersonalCredentials,
  credentialTarget,
}: {
  project: string;
  identity: string;
  canAdminister: boolean;
  onClose: () => void;
  onSaved: () => void;
  section?: "providers" | "models" | "mcp" | "credentials" | "routing";
  onCredentials?: (target?: CredentialTarget) => void;
  onPersonalCredentials?: (target?: CredentialTarget) => void;
  credentialTarget?: CredentialTarget | undefined;
}) {
  const path = `${projectPath(project)}/chat/settings`;
  const resource = useResource<ChatSettingsSnapshot>(
    canAdminister ? path : null,
    0,
    identity,
  );
  const [draft, setDraft] = useState<ChatSettingsWrite | null>(null);
  const [editor, setEditor] = useState<SettingsEditor | null>(null);
  const [saving, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [catalogBusy, setCatalogBusy] = useState(false);
  const busy = saving || catalogBusy;
  const scope = JSON.stringify([project, identity, canAdminister]);
  const currentScope = useRef(scope);
  const reconcilingRevision = useRef<{
    scope: string;
    revision: number;
    setup?: CredentialSetup | undefined;
  } | null>(null);
  currentScope.current = scope;
  useEffect(() => {
    reconcilingRevision.current = null;
    setBusy(false);
    setCatalogBusy(false);
    setError("");
    setNotice("");
  }, [scope]);
  useEffect(() => {
    if (resource.data) {
      setDraft(settingsWrite(resource.data));
      setEditor(null);
      if (
        reconcilingRevision.current?.scope === scope &&
        resource.data.revision >= reconcilingRevision.current.revision
      ) {
        const completed = reconcilingRevision.current;
        reconcilingRevision.current = null;
        setBusy(false);
        const setup = completed?.setup;
        if (setup?.authentication === "shared") onCredentials?.(setup.target);
        else if (setup) onPersonalCredentials?.(setup.target);
      }
    } else setDraft(null);
  }, [resource.data, scope]);
  useEffect(
    () => () => {
      currentScope.current = "";
    },
    [],
  );
  const changed = Boolean(
    draft &&
    resource.data &&
    JSON.stringify(draft) !== JSON.stringify(settingsWrite(resource.data)),
  );
  async function save(value?: ChatSettingsWrite, setup?: CredentialSetup) {
    if (!(value ?? draft) || !canAdminister || busy) return;
    const submitted = structuredClone(value ?? draft!),
      submittedScope = scope;
    setBusy(true);
    setError("");
    setNotice("");
    let reconciling = false;
    try {
      const saved = await request<{ revision: number }>(
        path,
        submitted,
        undefined,
        "PUT",
      );
      if (currentScope.current !== submittedScope) return;
      reconcilingRevision.current = {
        scope: submittedScope,
        revision: saved.revision,
        setup,
      };
      reconciling = true;
      setNotice("Project chat configuration saved.");
      resource.refresh();
      onSaved();
    } catch (e) {
      if (currentScope.current === submittedScope)
        setError(
          `${errorMessage(e)} Reload settings to reconcile before changing a submitted configuration.`,
        );
    } finally {
      if (currentScope.current === submittedScope && !reconciling)
        setBusy(false);
    }
  }
  function remove(kind: SettingsEditor["kind"], id: string) {
    if (!draft || busy) return;
    const next = structuredClone(draft);
    if (kind === "provider") {
      delete next.providers.profiles[id];
      if (next.labels) delete next.labels.providers[id];
      delete next.provider_authentication[id];
    } else if (kind === "model") {
      delete next.models.profiles[id];
      if (next.labels) delete next.labels.models[id];
    } else {
      delete next.mcp.servers[id];
      delete next.mcp_authentication[id];
    }
    setDraft(next);
  }
  function editorForm(kind: SettingsEditor["kind"]) {
    return (
      <>
        {editor && editor.kind === kind ? (
          <div
            hidden={
              editor.kind !==
              (
                {
                  providers: "provider",
                  models: "model",
                  mcp: "mcp",
                  credentials: null,
                  routing: null,
                } as const
              )[section]
            }
          >
            <ChatSettingsEditor
              key={`${editor.kind}:${editor.id}`}
              editor={editor}
              draft={draft!}
              project={project}
              saved={resource.data!}
              canManageCredentials={!changed}
              onCredentials={onCredentials}
              onPersonalCredentials={onPersonalCredentials}
              onBusyChange={(value) => {
                if (currentScope.current === scope) setCatalogBusy(value);
              }}
              onCancel={() => setEditor(null)}
              onApply={(value, setup) => {
                setDraft(value);
                setEditor(null);
                setNotice("");
                void save(value, setup);
              }}
            />
          </div>
        ) : null}
      </>
    );
  }
  function actionBar() {
    return (
      <div className="chat-settings-actions">
        {section !== "credentials" && onCredentials ? (
          <Button onClick={() => onCredentials()}>Manage credentials</Button>
        ) : null}
        <Button
          variant="primary"
          disabled={!changed || busy || Boolean(editor)}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save project configuration"}
        </Button>
        <Button
          disabled={busy}
          onClick={() => {
            resource.refresh();
            setError("");
            setNotice("");
          }}
        >
          Reload settings
        </Button>
        {changed ? (
          <Button
            disabled={busy}
            onClick={() => {
              if (resource.data) setDraft(settingsWrite(resource.data));
              setEditor(null);
              setError("");
              setNotice("Unsaved configuration changes discarded.");
            }}
          >
            Discard changes
          </Button>
        ) : null}
        <span className="field-help">
          {changed
            ? "Unsaved configuration changes"
            : `Revision ${draft?.revision}`}
        </span>
      </div>
    );
  }
  function inventory(
    kind: SettingsEditor["kind"],
    title: string,
    rows: InventoryRow[],
  ) {
    const inUse = (id: string) =>
      kind === "provider"
        ? Object.values(draft!.models.profiles).some(
            (model) => model.providerProfile === id,
          )
        : kind === "model"
          ? Object.values(draft!.models.roles).includes(id)
          : false;
    if (["provider", "model"].includes(kind))
      return (
        <CatalogInventory
          kind={kind === "provider" ? "provider" : "model"}
          headingLevel={2}
          summary={
            rows.length +
            " configured " +
            (kind === "provider" ? "providers" : "models")
          }
          busy={busy}
          editing={Boolean(editor)}
          onAdd={() => setEditor({ kind, id: "" })}
          footer={
            <>
              {actionBar()}
              <p className="field-help">
                Connection URLs must be allowed by your deployment
                administrator. Saving applies when chats are idle.
              </p>
            </>
          }
          rows={rows.map((row) => {
            const model = draft!.models.profiles[row.id];
            const provider = draft!.providers.profiles[row.id];
            const label =
              (kind === "provider"
                ? draft!.labels?.providers[row.id]
                : draft!.labels?.models[row.id]) ?? row.id;
            const models = Object.entries(draft!.models.profiles).filter(
              ([, item]) => item.providerProfile === row.id,
            );
            const roles = Object.entries(draft!.models.roles).filter(
              ([, profile]) => profile === row.id,
            );
            return {
              id: row.id,
              label,
              name: label,
              description:
                kind === "provider"
                  ? (provider?.baseUrl ?? "Codex subscription")
                  : (model?.model ?? row.detail),
              ...(kind === "provider" && provider
                ? {
                    icon: (
                      <ProviderIcon
                        provider={{
                          kind: provider.kind,
                          baseUrl: provider.baseUrl ?? null,
                        }}
                      />
                    ),
                  }
                : {}),
              searchText: row.id + " " + label + " " + row.detail,
              connection:
                kind === "model"
                  ? (draft!.labels?.providers[model?.providerProfile ?? ""] ??
                    model?.providerProfile ??
                    "")
                  : row.authentication === "shared"
                    ? "Project credential"
                    : row.authentication === "none"
                      ? "None"
                      : "Personal account",
              usage:
                kind === "provider"
                  ? models.length + (models.length === 1 ? " model" : " models")
                  : roles.length + " assigned roles",
              actionAriaLabel: "Edit " + row.id,
              mutationDisabled: Boolean(editor),
              onEdit: () => setEditor({ kind, id: row.id }),
              ...(!inUse(row.id)
                ? { onDelete: () => remove(kind, row.id) }
                : {}),
              details: (
                <div className="chat-settings-details">
                  <p>
                    Profile: <code>{row.id}</code>
                  </p>
                  <p>{row.detail}</p>
                  {kind === "provider" ? (
                    <>
                      <p>
                        {row.authentication === "shared"
                          ? "Shared project credential"
                          : row.authentication === "none"
                            ? "No authentication"
                            : "Each user connects their own account"}
                      </p>
                      {models.map(([id, value]) => (
                        <p key={id}>
                          {draft!.labels?.models[id] ?? id} · {value.model}
                        </p>
                      ))}
                      <Button
                        onClick={() => {
                          if (row.authentication === "shared")
                            onCredentials?.({ kind: "provider", id: row.id });
                          else
                            onPersonalCredentials?.({
                              kind: "provider",
                              id: row.id,
                            });
                        }}
                      >
                        Manage credentials
                      </Button>
                    </>
                  ) : (
                    <p>
                      {roles.map(([role]) => role).join(", ") ||
                        "Uses primary unless assigned to a role."}
                    </p>
                  )}
                </div>
              ),
            };
          })}
        >
          {editorForm(kind)}
        </CatalogInventory>
      );
    const columns: DataTableColumn<InventoryRow>[] = [
      {
        id: "name",
        label: "Profile",
        value: (row) => row.id,
        cell: (row) => (
          <span>
            <strong>{row.id}</strong>
            <small>{row.detail}</small>
          </span>
        ),
      },
      ...(kind === "model"
        ? []
        : [
            {
              id: "authentication",
              label: "Authentication",
              value: (row: InventoryRow) =>
                row.authentication === "shared"
                  ? "Shared project credential"
                  : row.authentication === "none"
                    ? "None"
                    : "Personal account",
            },
          ]),
      {
        id: "actions",
        label: "Actions",
        value: () => "",
        cell: (row) => (
          <div className="chat-settings-row-actions">
            <Button
              disabled={busy || Boolean(editor)}
              onClick={() => setEditor({ kind, id: row.id })}
            >
              Edit {row.id}
            </Button>
            <Button
              variant="danger"
              disabled={busy || Boolean(editor) || inUse(row.id)}
              title={
                inUse(row.id)
                  ? "Update model or role references before removing this profile."
                  : undefined
              }
              onClick={() => remove(kind, row.id)}
            >
              Remove
            </Button>
            {row.authentication === "shared" &&
            !(
              kind === "provider" &&
              draft!.providers.profiles[row.id]?.kind === "open_ai_codex"
            ) ? (
              <ChatSharedCredential
                key={`${scope}:${draft!.revision}:${row.id}`}
                path={`${path}/${kind === "provider" ? "providers" : "mcp"}/${encodeURIComponent(row.id)}/credential`}
                revision={draft!.revision}
                endpoint={
                  (kind === "provider"
                    ? draft!.providers.profiles[row.id]?.baseUrl
                    : draft!.mcp.servers[row.id]?.url) ?? ""
                }
                present={Boolean(
                  (kind === "provider"
                    ? resource.data?.credential_status.providers
                    : resource.data?.credential_status.mcp)?.[row.id],
                )}
                disabled={busy || changed || Boolean(editor)}
                onSaved={() => {
                  resource.refresh();
                  onSaved();
                  setNotice("Project credential saved.");
                }}
                onError={(e) => setError(errorMessage(e))}
              />
            ) : null}
          </div>
        ),
      },
    ];
    return (
      <section className="chat-settings-inventory" aria-label={title}>
        <header className="catalog-heading">
          <h3>{title}</h3>
          <Button
            disabled={busy || Boolean(editor)}
            onClick={() => setEditor({ kind, id: "" })}
          >
            Add {kind === "mcp" ? "MCP connection" : kind}
          </Button>
        </header>
        {editorForm(kind)}
        <DataTable
          data={rows}
          columns={columns}
          getRowId={(row) => row.id}
          label={title}
          itemLabel="profiles"
          search={{ columnId: "name", label: `Search ${title.toLowerCase()}` }}
        />
      </section>
    );
  }
  if (!canAdminister)
    return (
      <p role="alert">
        Project administration access is required to configure chat.
      </p>
    );
  return (
    <section
      className="chat-settings catalog-settings"
      aria-label="Project chat settings"
      data-section={section}
    >
      {!["providers", "models", "routing"].includes(section) ? (
        <header className="catalog-heading">
          <div>
            <h2>
              {section === "providers"
                ? "Providers"
                : section === "models"
                  ? "Models"
                  : section === "routing"
                    ? "Role routing"
                    : section === "credentials"
                      ? "Credentials"
                      : "MCP connections"}
            </h2>
            <p>
              {section === "providers"
                ? "Manage the model providers available to personal chats in this project."
                : section === "models"
                  ? "Configure model profiles and the roles that use them."
                  : section === "routing"
                    ? "Choose the primary model and specialized model assignments."
                    : section === "credentials"
                      ? "Save and replace the encrypted project credentials used by providers and MCP connections."
                      : "Manage remote tools. Each person connects their own account unless a project credential is explicitly configured."}
            </p>
          </div>
        </header>
      ) : null}
      {!["providers", "models"].includes(section) ? (
        <p className="field-help">
          Connection URLs must be allowed by your deployment administrator.
          Saving applies when chats are idle.
        </p>
      ) : null}
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <p className="alert" role="alert">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p role="status" className="sync-notice">
          {notice}
        </p>
      ) : null}
      {draft ? (
        <>
          {!["providers", "models"].includes(section) ? actionBar() : null}
          {editor &&
          editor.kind !==
            (
              {
                providers: "provider",
                models: "model",
                mcp: "mcp",
                credentials: null,
                routing: null,
              } as const
            )[section] ? (
            <p className="sync-notice" role="status">
              An editor is still open in{" "}
              {editor.kind === "provider"
                ? "Providers"
                : editor.kind === "model"
                  ? "Models"
                  : "MCP connections"}
              . Return there to finish, or cancel editing.
              <Button disabled={busy} onClick={() => setEditor(null)}>
                Cancel editing
              </Button>
            </p>
          ) : null}
          {section === "credentials" ? (
            <ChatProjectCredentials
              key={scope}
              path={path}
              identity={identity}
              disabled={busy || changed || Boolean(editor)}
              onSaved={() => {
                resource.refresh();
                onSaved();
                setNotice("Project credential saved.");
              }}
              initialConnection={credentialTarget}
            />
          ) : null}
          <div hidden={section !== "providers"}>
            {inventory(
              "provider",
              "Providers",
              Object.entries(draft.providers.profiles).map(([id, value]) => ({
                id,
                detail: `${value.kind}${value.baseUrl ? ` · ${value.baseUrl}` : ""}`,
                authentication: draft.provider_authentication[id] ?? "personal",
              })),
            )}
          </div>
          <div hidden={section !== "models"}>
            {inventory(
              "model",
              "Models",
              Object.entries(draft.models.profiles).map(([id, value]) => ({
                id,
                detail: `${value.model} · ${value.providerProfile} · ${
                  Object.entries(draft.models.roles)
                    .filter(([, profile]) => profile === id)
                    .map(([role]) => role)
                    .join(", ") || "No default roles"
                }`,
              })),
            )}
          </div>
          <div hidden={section !== "mcp"}>
            {inventory(
              "mcp",
              "MCP connections",
              Object.entries(draft.mcp.servers).map(([id, value]) => ({
                id,
                detail: `${value.url} · ${value.allowedTools.length} allowed tools`,
                authentication: draft.mcp_authentication[id] ?? "personal",
              })),
            )}
          </div>
          {section === "routing" ? (
            <div className="chat-role-settings">
              <ModelRoutingOverview
                roles={draft.models.roles}
                models={Object.entries(draft.models.profiles).map(
                  ([profile, model]) => ({
                    profile,
                    label: draft.labels?.models[profile] ?? profile,
                    model: model.model,
                    providerProfile: model.providerProfile,
                  }),
                )}
                disabled={busy || Boolean(editor)}
                onChange={(roles) =>
                  setDraft({ ...draft, models: { ...draft.models, roles } })
                }
              />
              <ModelRoleRouting
                roles={draft.models.roles}
                models={Object.entries(draft.models.profiles).map(
                  ([profile, model]) => ({
                    profile,
                    label: draft.labels?.models[profile] ?? profile,
                    model: model.model,
                    providerProfile: model.providerProfile,
                  }),
                )}
                disabled={busy || Boolean(editor)}
                onChange={(roles) =>
                  setDraft({ ...draft, models: { ...draft.models, roles } })
                }
              />
            </div>
          ) : null}
        </>
      ) : null}
    </section>
  );
}
