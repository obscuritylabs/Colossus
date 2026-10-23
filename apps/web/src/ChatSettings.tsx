import { CatalogInventory } from "@colossus/ui";
import { ProviderIcon } from "@colossus/ui/components/ProviderIcon";
import { ModelRoleRouting } from "@colossus/ui/components/ModelRoleRouting";
import { ModelRoutingOverview } from "@colossus/ui/components/ModelRoutingOverview";
import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { ApiFailure, projectPath, request } from "./api";
import { LoadState } from "./Home";
import { errorMessage, useResource } from "./resources";
import { ChatSettingsEditor, type SettingsEditor } from "./ChatSettingsEditors";
import { ConnectionSettingsExtras } from "./ConnectionSettingsExtras";
import { SettingsInheritancePanel } from "./SettingsInheritancePanel";
import type { ConnectionSettingsSection } from "./routes";
import {
  SettingsEditorPage,
  SettingsEditorUnavailable,
  type SettingsEditorNavigation,
} from "./settings-editor";
import {
  settingsWrite,
  type Authentication,
  type ChatSettingsSnapshot,
  type ChatSettingsWrite,
  type InheritedSection,
} from "./chat-settings";

interface InventoryRow {
  id: string;
  detail: string;
  authentication?: Authentication;
}

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
  settingsPath,
  instanceScope = false,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
}: SettingsEditorNavigation & {
  project: string;
  identity: string;
  canAdminister: boolean;
  onClose: () => void;
  onSaved: () => void;
  section?: ConnectionSettingsSection;
  settingsPath?: string;
  instanceScope?: boolean;
  onCredentials?: (target?: CredentialTarget) => void;
  onPersonalCredentials?: (target?: CredentialTarget) => void;
  credentialTarget?: CredentialTarget | undefined;
}) {
  const path = settingsPath ?? `${projectPath(project)}/chat/settings`;
  const resource = useResource<ChatSettingsSnapshot>(
    canAdminister ? path : null,
    0,
    identity,
  );
  const [saved, setSaved] = useState<ChatSettingsSnapshot | null>(null);
  const availability = saved ?? resource.data;
  const settingsEditable =
    canAdminister &&
    (instanceScope || availability?.execution?.settings_editable !== false);
  const executionReason = !settingsEditable
    ? (availability?.execution?.reason ??
      "Hosted execution is not enabled for this project. Ask a deployment operator to configure its runtime and allowed origins before editing project settings.")
    : "";
  const [draft, setDraft] = useState<ChatSettingsWrite | null>(null);
  const [mustReload, setMustReload] = useState(false);
  const mustReloadRef = useRef(false);
  const dirtyDraft = useRef(false);
  const minimumRevision = useRef<number | null>(null);
  const [inlineEditor, setEditor] = useState<SettingsEditor | null>(null);
  const [formDirty, setFormDirty] = useState(false);
  const [extraDirty, setExtraDirty] = useState(false);
  const routedKind = (
    { providers: "provider", models: "model", mcp: "mcp" } as const
  )[section as "providers" | "models" | "mcp"];
  const editor =
    editorLocation && routedKind
      ? {
          kind: routedKind,
          id: editorLocation.action === "edit" ? (editorLocation.id ?? "") : "",
        }
      : inlineEditor;
  function openEditor(value: SettingsEditor) {
    if (!settingsEditable || mustReloadRef.current) return;
    if (onOpenEditor)
      onOpenEditor(
        value.id ? { action: "edit", id: value.id } : { action: "new" },
      );
    else setEditor(value);
  }
  function closeEditor() {
    setEditor(null);
    if (!editorLocation) setFormDirty(false);
    onCloseEditor?.();
  }
  const [extraEditor, setExtraEditor] = useState<
    "search" | "variables" | "telemetry" | null
  >(null);
  const [resetExtraEditor, setResetExtraEditor] = useState(0);
  const editing = Boolean(editor) || Boolean(extraEditor);
  const extraSection = ["search", "telemetry", "variables"].includes(section);
  const [saving, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [catalogBusy, setCatalogBusy] = useState(false);
  const busy = saving || catalogBusy;
  const scope = JSON.stringify([
    path,
    project,
    identity,
    canAdminister,
    instanceScope,
  ]);
  const currentScope = useRef(scope);
  const operationScope = useRef({ scope });
  if (operationScope.current.scope !== scope)
    operationScope.current = { scope };
  const pendingSave = useRef<object | null>(null);
  currentScope.current = scope;
  useEffect(() => {
    setSaved(null);
    setDraft(null);
    mustReloadRef.current = false;
    setMustReload(false);
    minimumRevision.current = null;
    setBusy(false);
    setCatalogBusy(false);
    setError("");
    setNotice("");
    setExtraEditor(null);
    setFormDirty(false);
    setExtraDirty(false);
    setResetExtraEditor((value) => value + 1);
  }, [scope]);
  function acceptSettings(snapshot: ChatSettingsSnapshot) {
    setSaved(snapshot);
    setDraft(settingsWrite(snapshot));
    setEditor(null);
    setExtraEditor(null);
    setFormDirty(false);
    setExtraDirty(false);
    setResetExtraEditor((value) => value + 1);
    mustReloadRef.current = false;
    setMustReload(false);
    minimumRevision.current = null;
    onDirtyChange?.(false);
  }
  useEffect(() => {
    if (
      resource.data &&
      (!saved ||
        (!dirtyDraft.current && !pendingSave.current && !mustReloadRef.current))
    )
      acceptSettings(resource.data);
  }, [resource.data, scope]);
  useEffect(
    () => () => {
      currentScope.current = "";
    },
    [],
  );
  const changed = Boolean(
    draft &&
    saved &&
    JSON.stringify(draft) !== JSON.stringify(settingsWrite(saved)),
  );
  dirtyDraft.current = changed || formDirty || extraDirty;
  useEffect(() => {
    onDirtyChange?.(
      settingsEditable &&
        (changed || formDirty || extraDirty || saving || mustReload),
    );
  }, [
    settingsEditable,
    changed,
    formDirty,
    extraDirty,
    saving,
    mustReload,
    onDirtyChange,
  ]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  function requireReload() {
    mustReloadRef.current = true;
    setMustReload(true);
  }
  function currentOperation(operation: object, submittedScope: string) {
    return (
      currentScope.current === submittedScope &&
      operationScope.current === operation
    );
  }
  async function readCurrent(minimum: number) {
    const snapshot = await request<ChatSettingsSnapshot>(path);
    if (!Number.isSafeInteger(snapshot.revision) || snapshot.revision < minimum)
      throw new Error("The current saved revision is not available yet.");
    return snapshot;
  }
  async function reloadSettings() {
    if (busy || pendingSave.current === operationScope.current) return;
    if (
      (dirtyDraft.current || mustReloadRef.current) &&
      !window.confirm(
        "Discard your proposed configuration and reload current settings?",
      )
    )
      return;
    const submittedOperation = operationScope.current,
      submittedScope = scope;
    pendingSave.current = submittedOperation;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const snapshot = await readCurrent(
        minimumRevision.current ?? saved?.revision ?? 0,
      );
      if (!currentOperation(submittedOperation, submittedScope)) return;
      acceptSettings(snapshot);
      setNotice("Current settings loaded.");
    } catch (cause) {
      if (currentOperation(submittedOperation, submittedScope)) {
        requireReload();
        setError(
          `${errorMessage(cause)} Your proposed configuration is retained. Reload settings before saving again.`,
        );
      }
    } finally {
      if (pendingSave.current === submittedOperation)
        pendingSave.current = null;
      if (currentOperation(submittedOperation, submittedScope)) setBusy(false);
    }
  }
  async function save(value?: ChatSettingsWrite, setup?: CredentialSetup) {
    if (
      !(value ?? draft) ||
      !settingsEditable ||
      busy ||
      mustReloadRef.current ||
      pendingSave.current === operationScope.current
    )
      return;
    const submitted = structuredClone(value ?? draft!),
      submittedScope = scope,
      submittedOperation = operationScope.current;
    pendingSave.current = submittedOperation;
    setBusy(true);
    setError("");
    setNotice("");
    let accepted = false;
    try {
      const result = await request<{ revision: number }>(
        path,
        submitted,
        undefined,
        "PUT",
      );
      if (!currentOperation(submittedOperation, submittedScope)) return;
      accepted = true;
      if (
        !Number.isSafeInteger(result.revision) ||
        result.revision < submitted.revision
      )
        throw new Error(
          "The save acknowledgement did not include a current revision.",
        );
      minimumRevision.current = result.revision;
      const snapshot = await readCurrent(result.revision);
      if (!currentOperation(submittedOperation, submittedScope)) return;
      acceptSettings(snapshot);
      setNotice(
        instanceScope
          ? "Control Plane defaults published. Projects inherit these definitions; credentials remain explicitly assigned."
          : "Project configuration saved.",
      );
      onSaved();
      if (editorLocation && !setup) onCloseEditor?.();
      if (setup?.authentication === "shared") onCredentials?.(setup.target);
      else if (setup) onPersonalCredentials?.(setup.target);
    } catch (cause) {
      if (currentOperation(submittedOperation, submittedScope)) {
        const uncertain = !(cause instanceof ApiFailure) || cause.status >= 500;
        if (
          accepted ||
          uncertain ||
          (cause instanceof ApiFailure && cause.status === 409)
        ) {
          requireReload();
          setError(
            `${accepted ? "The submitted configuration was accepted, but its current saved revision could not be confirmed." : errorMessage(cause)} Your proposed configuration is retained. Reload settings before saving again.`,
          );
        } else setError(errorMessage(cause));
      }
    } finally {
      if (pendingSave.current === submittedOperation)
        pendingSave.current = null;
      if (currentOperation(submittedOperation, submittedScope)) setBusy(false);
    }
  }
  function recoveryControls() {
    return editorLocation && mustReload ? (
      <div className="chat-settings-actions">
        <Button disabled={busy} onClick={() => void reloadSettings()}>
          Reload settings
        </Button>
        <span className="field-help">
          Review the latest saved configuration before making another change.
        </span>
      </div>
    ) : null;
  }
  function resetInherited(kind: InheritedSection, id: string) {
    if (
      !settingsEditable ||
      !draft?.inheritance ||
      busy ||
      changed ||
      mustReloadRef.current
    )
      return;
    void save({
      ...draft,
      inheritance: {
        global_revision: draft.inheritance.global_revision,
        reset: { [kind]: [id] },
      },
    });
  }
  function remove(kind: SettingsEditor["kind"], id: string) {
    if (!settingsEditable || !draft || busy || mustReloadRef.current) return;
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
                  search: null,
                  telemetry: null,
                  variables: null,
                } as const
              )[section]
            }
          >
            <ChatSettingsEditor
              key={`${editor.kind}:${editor.id}:${resetExtraEditor}`}
              editor={editor}
              draft={draft!}
              project={project}
              settingsPath={path}
              instanceScope={instanceScope}
              saved={{
                ...saved!,
                credential_status: saved?.credential_status ?? {
                  providers: {},
                  mcp: {},
                },
              }}
              busy={busy || mustReload || !settingsEditable}
              onDirtyChange={setFormDirty}
              canManageCredentials={!changed && !formDirty && !mustReload}
              onCredentials={
                onCredentials
                  ? (target) => {
                      if (changed || formDirty || mustReloadRef.current) return;
                      setEditor(null);
                      onDirtyChange?.(false);
                      onCredentials(target);
                    }
                  : undefined
              }
              onPersonalCredentials={
                onPersonalCredentials
                  ? (target) => {
                      if (changed || formDirty || mustReloadRef.current) return;
                      setEditor(null);
                      onDirtyChange?.(false);
                      onPersonalCredentials(target);
                    }
                  : undefined
              }
              onBusyChange={(value) => {
                if (currentScope.current === scope) setCatalogBusy(value);
              }}
              onCancel={closeEditor}
              onApply={(value, setup) => {
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
        {section !== "credentials" &&
        section !== "telemetry" &&
        section !== "variables" &&
        onCredentials ? (
          <Button
            disabled={
              busy || mustReload || !settingsEditable || editing || changed
            }
            onClick={() => onCredentials()}
          >
            Manage credentials
          </Button>
        ) : null}
        <Button
          variant="primary"
          disabled={
            !settingsEditable || !changed || busy || mustReload || editing
          }
          onClick={() => void save()}
        >
          {busy
            ? "Saving…"
            : instanceScope
              ? "Publish Control Plane defaults"
              : "Save project configuration"}
        </Button>
        <Button
          disabled={busy || (!mustReload && (editing || changed))}
          onClick={() => void reloadSettings()}
        >
          Reload settings
        </Button>
        {changed ? (
          <Button
            disabled={busy || mustReload}
            onClick={() => {
              if (saved) setDraft(settingsWrite(saved));
              setEditor(null);
              setExtraEditor(null);
              setFormDirty(false);
              setExtraDirty(false);
              setResetExtraEditor((value) => value + 1);
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
          description={
            instanceScope
              ? kind === "provider"
                ? "Control Plane provider definitions flow into projects. Credential access requires a separate assignment."
                : "Publish model profiles and their provider references for projects to inherit. Assign each model to the roles that use it."
              : kind === "provider"
                ? "Provider connections available to chats in this project. Each person connects privately unless a shared credential is explicitly assigned."
                : "Model profiles available to this project and the roles that use them."
          }
          usageLabel={
            kind === "provider" ? "Configured models" : "Assigned roles"
          }
          headingLevel={2}
          summary={
            rows.length +
            " configured " +
            (kind === "provider" ? "providers" : "models")
          }
          busy={busy || mustReload || !settingsEditable}
          editing={editing}
          onAdd={() => openEditor({ kind, id: "" })}
          footer={
            <>
              {actionBar()}
              <p className="field-help">
                {kind === "provider"
                  ? "Connection URLs must be allowed by your deployment administrator. Saving applies when chats are idle."
                  : "Model profiles reference configured providers. Saving applies when chats are idle."}
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
            const inheritedKind = kind === "provider" ? "providers" : "models";
            const origin = instanceScope
              ? "Control Plane definition"
              : saved?.inheritance?.provenance[inheritedKind][row.id] ===
                  "global"
                ? "Inherited from Control Plane"
                : saved?.inheritance?.global_entries?.[inheritedKind].includes(
                      row.id,
                    )
                  ? "Project override"
                  : "Project definition";
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
              connection: (
                <span className="catalog-connection-copy">
                  <span>
                    {kind === "model"
                      ? (draft!.labels?.providers[
                          model?.providerProfile ?? ""
                        ] ??
                        model?.providerProfile ??
                        "")
                      : row.authentication === "shared"
                        ? "Explicit shared credential"
                        : row.authentication === "none"
                          ? "None"
                          : "Personal account"}
                  </span>
                  <small className="field-help">{origin}</small>
                </span>
              ),
              usage:
                kind === "provider"
                  ? models.length + (models.length === 1 ? " model" : " models")
                  : roles.length + " assigned roles",
              actionAriaLabel: "Edit " + row.id,
              actionLabel:
                origin === "Inherited from Control Plane"
                  ? "Customize"
                  : "Edit",
              mutationDisabled: editing,
              onEdit: () => openEditor({ kind, id: row.id }),
              ...(!inUse(row.id)
                ? { onDelete: () => remove(kind, row.id) }
                : {}),
              details: (
                <div className="chat-settings-details">
                  <p>
                    Profile: <code>{row.id}</code>
                  </p>
                  <p>{row.detail}</p>
                  <p className="field-help">{origin}</p>
                  {kind === "provider" ? (
                    <>
                      <p>
                        {row.authentication === "shared"
                          ? "Explicit shared credential"
                          : row.authentication === "none"
                            ? "No authentication"
                            : "Each user connects their own account"}
                      </p>
                      {models.map(([id, value]) => (
                        <p key={id}>
                          {draft!.labels?.models[id] ?? id} · {value.model}
                        </p>
                      ))}
                      {row.authentication !== "none" &&
                      (row.authentication === "shared"
                        ? onCredentials
                        : onPersonalCredentials) ? (
                        <Button
                          disabled={
                            busy ||
                            mustReload ||
                            !settingsEditable ||
                            changed ||
                            editing
                          }
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
                      ) : null}
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
                  ? "Explicit shared credential"
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
              disabled={busy || mustReload || !settingsEditable || editing}
              onClick={() => openEditor({ kind, id: row.id })}
            >
              Edit {row.id}
            </Button>
            <Button
              variant="danger"
              disabled={
                busy ||
                mustReload ||
                !settingsEditable ||
                editing ||
                inUse(row.id)
              }
              title={
                inUse(row.id)
                  ? "Update model or role references before removing this profile."
                  : undefined
              }
              onClick={() => remove(kind, row.id)}
            >
              Remove
            </Button>
            {!instanceScope &&
            row.authentication === "shared" &&
            onCredentials ? (
              <Button
                disabled={
                  busy || mustReload || !settingsEditable || changed || editing
                }
                onClick={() => onCredentials({ kind: "mcp", id: row.id })}
              >
                Manage shared credential
              </Button>
            ) : null}
          </div>
        ),
      },
    ];
    return (
      <section className="chat-settings-inventory" aria-label={title}>
        <header className="catalog-heading">
          <div>
            <h2>{title}</h2>
            <p>
              Manage remote tools and choose personal or explicitly assigned
              shared authentication.
            </p>
          </div>
          <Button
            disabled={busy || mustReload || !settingsEditable || editing}
            onClick={() => openEditor({ kind, id: "" })}
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
        {actionBar()}
        <p className="field-help">
          Connection URLs must be allowed by your deployment administrator.
          Saving applies when chats are idle.
        </p>
      </section>
    );
  }
  if (!canAdminister)
    return (
      <p role="alert">
        Project administration access is required to configure chat.
      </p>
    );
  if (editorLocation && !settingsEditable && saved)
    return (
      <SettingsEditorUnavailable
        backLabel={
          section === "mcp"
            ? "MCP servers"
            : section[0]!.toUpperCase() + section.slice(1)
        }
        onBack={closeEditor}
        reason={executionReason}
      />
    );
  if (editorLocation && routedKind) {
    const backLabel =
      section === "mcp"
        ? "MCP servers"
        : section === "models"
          ? "Models"
          : "Providers";
    const entries =
      routedKind === "provider"
        ? draft?.providers.profiles
        : routedKind === "model"
          ? draft?.models.profiles
          : draft?.mcp.servers;
    const unavailable =
      editorLocation.action === "edit" &&
      (!editorLocation.id ||
        (entries && !Object.hasOwn(entries, editorLocation.id)));
    return (
      <section className="chat-settings catalog-settings">
        <LoadState {...resource} retry={resource.refresh} />
        {error ? (
          <p role="alert" className="alert">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p role="status" className="sync-notice">
            {notice}
          </p>
        ) : null}
        {recoveryControls()}
        {draft ? (
          unavailable ? (
            <SettingsEditorUnavailable
              backLabel={backLabel}
              onBack={closeEditor}
            />
          ) : (
            <SettingsEditorPage
              title={`${editorLocation.action === "new" ? "Add" : "Edit"} ${routedKind === "mcp" ? "MCP server" : routedKind}`}
              backLabel={backLabel}
              showHeading={false}
              busy={busy}
              onBack={closeEditor}
              description={
                instanceScope
                  ? "Control Plane defaults flow into projects. Credential access requires a separate explicit assignment."
                  : "Changes apply to this project. Editing an inherited definition creates a project override."
              }
            >
              {editorForm(routedKind)}
            </SettingsEditorPage>
          )
        ) : null}
      </section>
    );
  }
  return (
    <section
      className="chat-settings catalog-settings"
      aria-label={
        instanceScope ? "Control Plane configuration" : "Project chat settings"
      }
      data-section={section}
    >
      <LoadState {...resource} retry={resource.refresh} />
      {executionReason && saved ? (
        <div className="sync-notice">
          <p role="status">{executionReason}</p>
          {section === "credentials" ? (
            <Button onClick={resource.refresh}>Reload settings</Button>
          ) : null}
        </div>
      ) : null}
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
      {recoveryControls()}
      {draft ? (
        <>
          {!["providers", "models", "mcp", "telemetry", "credentials"].includes(
            section,
          ) &&
          !extraSection &&
          (!onOpenEditor || section !== "routing")
            ? actionBar()
            : null}
          {editor &&
          editor.kind !==
            (
              {
                providers: "provider",
                models: "model",
                mcp: "mcp",
                credentials: null,
                routing: null,
                search: null,
                telemetry: null,
                variables: null,
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
          {section === "credentials" && settingsEditable ? (
            <ChatProjectCredentials
              key={scope}
              path={path}
              identity={identity}
              disabled={
                busy || mustReload || !settingsEditable || changed || editing
              }
              onSaved={() => {
                resource.refresh();
                onSaved();
                setNotice("Project credential saved.");
              }}
              initialConnection={credentialTarget}
              editorLocation={editorLocation}
              onOpenEditor={onOpenEditor}
              onCloseEditor={onCloseEditor}
              onDirtyChange={onDirtyChange}
            />
          ) : null}
          {!editorLocation || extraSection ? (
            <div
              hidden={!["search", "telemetry", "variables"].includes(section)}
            >
              <ConnectionSettingsExtras
                section={
                  section === "telemetry" || section === "variables"
                    ? section
                    : "search"
                }
                draft={draft}
                saved={saved!}
                instanceScope={instanceScope}
                busy={
                  busy ||
                  mustReload ||
                  !settingsEditable ||
                  Boolean(editor) ||
                  Boolean(extraEditor && extraEditor !== section)
                }
                navigationBusy={busy}
                onEditingChange={setExtraEditor}
                onDirtyChange={setExtraDirty}
                resetEditor={resetExtraEditor}
                editorLocation={editorLocation}
                onOpenEditor={onOpenEditor}
                onCloseEditor={onCloseEditor}
                onChange={setDraft}
                onSave={(value) => void save(value)}
                onCredentials={onCredentials}
                onPersonalCredentials={onPersonalCredentials}
              />
            </div>
          ) : null}
          {!editorLocation && extraSection && section !== "telemetry" ? (
            <>
              {actionBar()}
              {section === "search" ? (
                <p className="field-help">
                  Search endpoints must be allowed by your deployment
                  administrator. Saving applies when chats are idle.
                </p>
              ) : null}
            </>
          ) : null}
          {!editorLocation && section === "telemetry" ? (
            <Button
              disabled={busy || (!mustReload && editing)}
              onClick={() => void reloadSettings()}
            >
              Reload settings
            </Button>
          ) : null}
          {extraEditor && extraEditor !== section ? (
            <p className="sync-notice" role="status">
              An editor is still open in{" "}
              {extraEditor === "search"
                ? "Search"
                : extraEditor === "variables"
                  ? "Variables"
                  : "Telemetry"}
              . Return there to finish, or cancel editing.
              <Button
                disabled={busy}
                onClick={() => {
                  setExtraEditor(null);
                  setResetExtraEditor((value) => value + 1);
                }}
              >
                Cancel editing
              </Button>
            </p>
          ) : null}
          {!editorLocation ? (
            <>
              <div hidden={section !== "providers"}>
                {inventory(
                  "provider",
                  "Providers",
                  Object.entries(draft.providers.profiles).map(
                    ([id, value]) => ({
                      id,
                      detail: `${value.kind}${value.baseUrl ? ` · ${value.baseUrl}` : ""}`,
                      authentication:
                        draft.provider_authentication[id] ?? "personal",
                    }),
                  ),
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
            </>
          ) : null}
          {section === "routing" ? (
            <>
              {onOpenEditor && !editorLocation ? (
                <header className="catalog-heading">
                  <div>
                    <h2>Model routing</h2>
                    <p>
                      Choose the models used by each role. Unassigned roles use
                      the primary model.
                    </p>
                  </div>
                  <Button
                    disabled={
                      busy ||
                      mustReload ||
                      !settingsEditable ||
                      changed ||
                      editing
                    }
                    onClick={() => onOpenEditor({ action: "edit" })}
                  >
                    Configure routing
                  </Button>
                </header>
              ) : null}
              {editorLocation ? (
                <Button disabled={busy} onClick={closeEditor}>
                  Back to Model routing
                </Button>
              ) : null}
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
                  disabled={
                    busy ||
                    mustReload ||
                    !settingsEditable ||
                    editing ||
                    Boolean(onOpenEditor && !editorLocation)
                  }
                  onChange={(roles) =>
                    setDraft({ ...draft, models: { ...draft.models, roles } })
                  }
                />
                {!onOpenEditor || editorLocation ? (
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
                    disabled={
                      busy ||
                      !settingsEditable ||
                      editing ||
                      Boolean(onOpenEditor && !editorLocation)
                    }
                    onChange={(roles) =>
                      setDraft({ ...draft, models: { ...draft.models, roles } })
                    }
                  />
                ) : null}
              </div>
              {editorLocation ? (
                <div className="chat-settings-actions">
                  <Button disabled={busy} onClick={closeEditor}>
                    Cancel
                  </Button>
                  <Button
                    variant="primary"
                    disabled={
                      busy || mustReload || !settingsEditable || !changed
                    }
                    onClick={() => void save()}
                  >
                    Save routing
                  </Button>
                </div>
              ) : null}
            </>
          ) : null}
          {!editorLocation && !instanceScope && saved ? (
            <SettingsInheritancePanel
              saved={saved}
              section={section}
              busy={
                busy || mustReload || !settingsEditable || changed || editing
              }
              onReset={resetInherited}
            />
          ) : null}
          {!editorLocation &&
          !instanceScope &&
          saved &&
          section === "search" ? (
            <SettingsInheritancePanel
              saved={saved}
              section="routing"
              roleFamily="search"
              busy={
                busy || mustReload || !settingsEditable || changed || editing
              }
              onReset={resetInherited}
            />
          ) : null}
        </>
      ) : null}
    </section>
  );
}
