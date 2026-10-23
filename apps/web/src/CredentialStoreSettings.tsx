import type { CredentialTarget, CredentialExecution } from "./credential-types";
import {
  SettingsEditorPage,
  SettingsEditorUnavailable,
  type SettingsEditorLocation,
} from "./settings-editor";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";
import {
  Button,
  CatalogInventory,
  DropdownSelect,
  TextInput,
} from "@colossus/ui";
import { CredentialEditor } from "@colossus/ui/components/CredentialEditor";
import { CredentialForm } from "@colossus/ui/components/CredentialForm";
import { IconLock, IconRefresh } from "@tabler/icons-react";
import {
  ToastRegion,
  useToastQueue,
} from "@colossus/ui/components/ToastRegion";
import { request } from "./api";
import { useResource, errorMessage } from "./resources";
import { LoadState } from "./Home";

type Kind = "api_key" | "bearer_token" | "client_secret" | "generic_secret";
interface Destination {
  id: string;
  kind: "provider" | "mcp" | "search";
  label: string;
  endpoint: string;
  credential_binding: string;
  credential_id?: string | null;
  project_id?: string;
  credential_source?: "personal" | "project" | "global" | "none";
}
interface Credential {
  id: string;
  label: string;
  kind: Kind;
  legacy: boolean;
  used_by: Destination[];
}
export interface CredentialSnapshot {
  execution?: CredentialExecution;
  revision: number;
  credentials: Credential[];
  connections: Destination[];
}
const kinds: Record<Kind, string> = {
  api_key: "API key",
  bearer_token: "Bearer token",
  client_secret: "OAuth client secret",
  generic_secret: "Generic secret",
};
const destinationKey = (row: Destination) => `${row.kind}:${row.id}`;

/** A routed page already provides its heading; retain the shared form geometry. */
function CredentialEntryEditor({
  page,
  children,
  onCancel,
  busy,
  ...props
}: ComponentProps<typeof CredentialEditor> & { page: boolean }) {
  if (!page)
    return (
      <CredentialEditor {...props} busy={busy} onCancel={onCancel}>
        {children}
      </CredentialEditor>
    );
  return (
    <section
      className="shared-credential-editor shared-provider-settings-editor"
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented && !busy) {
          event.preventDefault();
          onCancel();
        }
      }}
    >
      <div className="shared-credential-editor-body provider-editor-section provider-credential-control">
        {children}
      </div>
      <footer className="mcp-editor-actions">
        <Button disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
      </footer>
    </section>
  );
}

/** The host owns storage and destination authority; saved values never enter this view. */
export function CredentialStoreSettings({
  path,
  identity,
  canEdit,
  projectScope = false,
  globalScope = false,
  resourcePath = path,
  onSaved,
  initialConnection,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
  assignmentContext,
}: {
  path: string;
  identity: string;
  canEdit: boolean;
  projectScope?: boolean;
  globalScope?: boolean;
  resourcePath?: string;
  onSaved: () => void;
  initialConnection?: CredentialTarget | undefined;
  editorLocation?: SettingsEditorLocation | undefined;
  onOpenEditor?: ((location: SettingsEditorLocation) => void) | undefined;
  onCloseEditor?: (() => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  assignmentContext?: ReactNode;
}) {
  const resource = useResource<CredentialSnapshot>(resourcePath, 0, identity);
  const [editor, setEditor] = useState<"secret" | "attach" | "remove" | null>(
    null,
  );
  const [id, setId] = useState("");
  const [label, setLabel] = useState("");
  const labelHelp = useId();
  const savedLabel = label.trim();
  const labelError = !savedLabel
    ? "Enter a display label."
    : new TextEncoder().encode(savedLabel).length > 160 ||
        /[\u0000-\u001f\u007f-\u009f]/u.test(savedLabel)
      ? "Display label must contain 1–160 UTF-8 bytes and no control characters."
      : "";
  const [kind, setKind] = useState<Kind>("api_key");
  const [target, setTarget] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const { toasts, pushToast, dismissToast, clearToasts } = useToastQueue();
  const [saving, setSaving] = useState(false);
  const [waitingRevision, setWaitingRevision] = useState(0);
  const [dirty, setDirty] = useState(false);
  const [mustReload, setMustReload] = useState(false);
  const [readyRoute, setReadyRoute] = useState("");
  const reviewed = useRef<CredentialSnapshot | null>(null);
  const pending = useRef<object | null>(null);
  const scope = JSON.stringify([path, resourcePath, identity, canEdit]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const opened = useRef("");
  const routeKey = JSON.stringify([scope, editorLocation]);
  const routed = Boolean(editorLocation);
  const view = JSON.stringify([scope, editorLocation]);
  const operationScope = useRef({ view });
  if (operationScope.current.view !== view) operationScope.current = { view };
  useEffect(() => {
    onDirtyChange?.(dirty || saving);
    return () => onDirtyChange?.(false);
  }, [dirty, saving, onDirtyChange]);
  useEffect(() => {
    setEditor(null);
    setId("");
    setLabel("");
    setTarget("");
    setError("");
    setNotice("");
    clearToasts();
    setSaving(false);
    setWaitingRevision(0);
    setDirty(false);
    setMustReload(false);
    setReadyRoute("");
    reviewed.current = null;
    opened.current = "";
  }, [scope, clearToasts]);
  useEffect(
    () => () => {
      currentScope.current = "";
    },
    [],
  );
  const data = resource.data;
  const executionUnavailable = data?.execution?.settings_editable === false;
  const executionReason =
    data?.execution?.reason ||
    "A deployment operator must provision this project's hosted runtime before credentials can be configured.";
  const busy =
    saving ||
    mustReload ||
    executionUnavailable ||
    !canEdit ||
    resource.loading ||
    Boolean(waitingRevision && (!data || data.revision < waitingRevision));
  const credential = data?.credentials.find((row) => row.id === id);
  const connection = (reviewed.current ?? data)?.connections.find(
    (row) => destinationKey(row) === target,
  );
  function closeEditor() {
    if (saving) return;
    if (routed && onCloseEditor) onCloseEditor();
    else {
      setEditor(null);
      setDirty(false);
      reviewed.current = null;
    }
  }
  function edit(value?: Credential, destination?: Destination) {
    if (executionUnavailable || !canEdit) return;
    if (onOpenEditor) {
      onOpenEditor(
        value
          ? { action: "edit", id: value.id }
          : {
              action: "new",
              ...(destination
                ? { target: { kind: destination.kind, id: destination.id } }
                : {}),
            },
      );
      return;
    }
    reviewed.current = data;
    setDirty(false);
    setMustReload(false);
    setId(value?.id ?? "");
    setLabel(value?.label ?? "");
    setKind(value?.kind ?? "api_key");
    setTarget(destination ? destinationKey(destination) : "");
    setEditor("secret");
    setError("");
    setNotice("");
  }
  useEffect(() => {
    if (!editorLocation) {
      if (readyRoute) {
        setEditor(null);
        setDirty(false);
        setReadyRoute("");
      }
      return;
    }
    if (!data || readyRoute === routeKey) return;
    const selected =
      editorLocation.action === "edit"
        ? data.credentials.find((row) => row.id === editorLocation.id)
        : undefined;
    reviewed.current = data;
    setId(selected?.id ?? "");
    setLabel(selected?.label ?? "");
    setKind(selected?.kind ?? "api_key");
    const destination =
      editorLocation.action === "new" && editorLocation.target
        ? data.connections.find(
            (row) =>
              row.kind === editorLocation.target!.kind &&
              row.id === editorLocation.target!.id,
          )
        : undefined;
    setTarget(destination ? destinationKey(destination) : "");
    setEditor("secret");
    setError("");
    setNotice("");
    setDirty(false);
    setMustReload(false);
    setReadyRoute(routeKey);
    setSaving(false);
  }, [data, editorLocation, routeKey, readyRoute]);
  useEffect(() => {
    const key = JSON.stringify([scope, initialConnection]);
    if (
      routed ||
      !initialConnection ||
      !data ||
      !canEdit ||
      executionUnavailable ||
      opened.current === key
    )
      return;
    opened.current = key;
    const destination = data.connections.find(
      (row) =>
        row.kind === initialConnection.kind && row.id === initialConnection.id,
    );
    if (!globalScope && destination?.credential_source === "global") {
      setNotice(
        "This connection uses a credential assigned by a Control Plane administrator. Ask that administrator to change its assignment.",
      );
      return;
    }
    const existing = data.credentials.find(
      (row) => row.id === destination?.credential_id,
    );
    if (onOpenEditor && destination) {
      edit(existing, destination);
      return;
    }
    if (destination && !existing && data.credentials.length) {
      reviewed.current = data;
      setId("");
      setTarget(destinationKey(destination));
      setEditor("attach");
    } else edit(existing, destination);
  }, [
    scope,
    initialConnection,
    data,
    canEdit,
    globalScope,
    routed,
    executionUnavailable,
    onOpenEditor,
  ]);
  async function mutate(url: string, body: object, method?: "DELETE") {
    if (
      !canEdit ||
      !data ||
      executionUnavailable ||
      mustReload ||
      pending.current === operationScope.current
    )
      return;
    const submitted = scope;
    const submittedOperation = operationScope.current;
    pending.current = submittedOperation;
    setSaving(true);
    setError("");
    setNotice("");
    try {
      const result = await request<{ revision: number }>(
        url,
        body,
        undefined,
        method,
      );
      if (currentScope.current !== submitted) return;
      resource.refresh();
      if (operationScope.current !== submittedOperation) return;
      setWaitingRevision(result.revision);
      setEditor(null);
      setDirty(false);
      reviewed.current = null;
      onDirtyChange?.(false);
      if (!routed) pushToast("Credential changes saved.");
      if (routed) onCloseEditor?.();
      onSaved();
    } catch (e) {
      if (
        currentScope.current === submitted &&
        operationScope.current === submittedOperation
      ) {
        setMustReload(true);
        setError(
          `${errorMessage(e)} Return to credentials and reload them before retrying.`,
        );
        resource.refresh();
      }
      throw e;
    } finally {
      if (pending.current === submittedOperation) pending.current = null;
      if (
        currentScope.current === submitted &&
        operationScope.current === submittedOperation
      )
        setSaving(false);
    }
  }
  async function attach(value: string | null, destination: Destination) {
    if (!data) return;
    await mutate(
      `${path}/attachments/${globalScope ? `${encodeURIComponent(destination.project_id ?? "")}/` : ""}${destination.kind}/${encodeURIComponent(destination.id)}`,
      {
        revision: (reviewed.current ?? data).revision,
        credential_id: value,
        credential_binding: destination.credential_binding,
      },
    );
  }
  const inventoryLabel =
    globalScope || projectScope ? "secrets" : "credentials";
  const description = globalScope
    ? "Encrypted instance secrets are used only by the project connections an administrator explicitly assigns."
    : projectScope
      ? "Shared within this project. Only explicitly attached connections may use a saved credential."
      : "Private to your account in this project. Save a credential now and attach it to a provider, search service, or MCP server later.";
  const destinationOptions = (
    <>
      <option value="">
        {editor === "attach" ? "Choose a connection" : "Save without attaching"}
      </option>
      {((reviewed.current ?? data)?.connections ?? []).map((row) => (
        <option
          key={destinationKey(row)}
          value={destinationKey(row)}
          disabled={!globalScope && row.credential_source === "global"}
        >
          {row.kind === "provider"
            ? "Provider"
            : row.kind === "search"
              ? "Search"
              : "MCP"}
          : {row.label}
          {!globalScope && row.credential_source === "global"
            ? " (assigned by Control Plane)"
            : ""}
        </option>
      ))}
    </>
  );
  const editorContent = (
    <>
      {editor === "secret" ? (
        <CredentialEntryEditor
          page={routed}
          title={id ? "Rotate credential" : "Add credential"}
          description={description}
          busy={saving}
          onCancel={closeEditor}
        >
          <label>
            <span>Display label</span>
            <TextInput
              aria-label="Display label"
              value={label}
              maxLength={160}
              disabled={busy}
              aria-describedby={labelHelp}
              aria-invalid={Boolean(labelError && dirty)}
              onChange={(event) => {
                setLabel(event.target.value);
                setDirty(true);
              }}
              placeholder="For example, GitHub workspace token"
            />
            <small
              id={labelHelp}
              role={labelError && dirty ? "alert" : undefined}
            >
              {labelError && dirty
                ? labelError
                : "Use 1–160 UTF-8 bytes without control characters."}
            </small>
          </label>
          <label>
            <span>Credential type</span>
            <DropdownSelect
              aria-label="Credential type"
              value={kind}
              disabled={busy}
              onChange={(event) => {
                setKind(event.target.value as Kind);
                setDirty(true);
              }}
            >
              {Object.entries(kinds).map(([value, text]) => (
                <option key={value} value={value}>
                  {text}
                </option>
              ))}
            </DropdownSelect>
          </label>
          {!id ? (
            <label>
              <span>Attach to connection</span>
              <DropdownSelect
                aria-label="Credential connection"
                value={target}
                disabled={busy}
                onChange={(event) => {
                  setTarget(event.target.value);
                  setDirty(true);
                }}
              >
                {destinationOptions}
              </DropdownSelect>
            </label>
          ) : (
            <p className="field-help">
              Rotation replaces the saved value. Existing connection assignments
              stay in place; use Attach existing credential to change an
              assignment.
            </p>
          )}
          {id && globalScope ? (
            <p className="field-help">
              Rotation updates this secret for every project where it is
              assigned. The connections shown here are limited to the selected
              project.
            </p>
          ) : id && credential?.used_by.length ? (
            <p className="field-help">
              Rotation updates every connection using this credential:{" "}
              {credential.used_by.map((row) => row.label).join(", ")}.
            </p>
          ) : null}
          <div onInputCapture={() => setDirty(true)}>
            <CredentialForm
              key={JSON.stringify([
                scope,
                id,
                target,
                reviewed.current?.revision,
              ])}
              scope={JSON.stringify([
                scope,
                id,
                target,
                reviewed.current?.revision,
              ])}
              label="API key or token"
              action={id ? "Save rotated credential" : "Save credential"}
              endpoint={
                connection?.endpoint ??
                (id
                  ? "Existing credential assignments"
                  : "Not attached to a connection")
              }
              disabled={busy}
              submitDisabled={Boolean(labelError)}
              description={
                connection
                  ? "Saving attaches this credential to the destination shown above."
                  : id
                    ? "Saving replaces the stored value for every existing assignment."
                    : "Saving alone gives agents no access to this credential."
              }
              onSave={async (secret) =>
                mutate(id ? `${path}/${encodeURIComponent(id)}` : path, {
                  revision: (reviewed.current ?? data)!.revision,
                  label: savedLabel,
                  kind,
                  secret,
                  ...(connection
                    ? {
                        attachment: {
                          ...(globalScope
                            ? { project_id: connection.project_id }
                            : {}),
                          kind: connection.kind,
                          connection: connection.id,
                          credential_binding: connection.credential_binding,
                        },
                      }
                    : {}),
                })
              }
              onSaved={() => {}}
              onError={() => {}}
            />
          </div>
        </CredentialEntryEditor>
      ) : null}
      {editor === "attach" ? (
        <CredentialEditor
          title="Attach credential"
          description="Choose a saved credential and review the destination that will receive it."
          busy={saving}
          onCancel={closeEditor}
          submitLabel="Attach credential"
          submitDisabled={busy || !id || !connection}
          onSubmit={() => {
            if (connection) void attach(id, connection).catch(() => {});
          }}
        >
          <label>
            <span>Credential</span>
            <DropdownSelect
              aria-label="Saved credential"
              value={id}
              disabled={busy}
              onChange={(event) => {
                setId(event.target.value);
                setDirty(true);
              }}
            >
              <option value="">Choose a credential</option>
              {(data?.credentials ?? []).map((row) => (
                <option key={row.id} value={row.id}>
                  {row.label}
                </option>
              ))}
            </DropdownSelect>
          </label>
          <label>
            <span>Connection</span>
            <DropdownSelect
              aria-label="Credential connection"
              value={target}
              disabled={busy}
              onChange={(event) => {
                setTarget(event.target.value);
                setDirty(true);
              }}
            >
              {destinationOptions}
            </DropdownSelect>
          </label>
          <Button
            disabled={busy}
            onClick={() => {
              setId("");
              setLabel("");
              setKind("api_key");
              setEditor("secret");
            }}
          >
            Create new credential
          </Button>
          {connection ? (
            <p>
              Credential destination: {connection.endpoint}
              {connection.credential_id && connection.credential_id !== id
                ? ". This replaces its current credential attachment."
                : ""}
            </p>
          ) : null}
        </CredentialEditor>
      ) : null}
      {editor === "remove" && credential ? (
        <CredentialEditor
          title="Remove credential"
          description={`Remove ${credential.label} from secure storage.`}
          busy={saving}
          onCancel={closeEditor}
          submitLabel="Remove credential"
          submitDisabled={busy}
          onSubmit={() =>
            void mutate(
              `${path}/${encodeURIComponent(id)}`,
              { revision: (reviewed.current ?? data)!.revision },
              "DELETE",
            ).catch(() => {})
          }
        >
          <p>
            {globalScope
              ? "Removal disconnects this secret from every project where it is assigned. Those connections will need another credential."
              : credential.used_by.length
                ? `These connections will need another credential: ${credential.used_by.map((row) => row.label).join(", ")}.`
                : "This credential is not attached to a connection."}
          </p>
        </CredentialEditor>
      ) : null}
    </>
  );
  if (editorLocation) {
    const destination =
      editorLocation.target &&
      data?.connections.find(
        (row) =>
          row.kind === editorLocation.target!.kind &&
          row.id === editorLocation.target!.id,
      );
    if (
      data &&
      !executionUnavailable &&
      editorLocation.target &&
      (!destination ||
        (!globalScope && destination.credential_source === "global"))
    )
      return (
        <SettingsEditorUnavailable
          backLabel={inventoryLabel}
          onBack={closeEditor}
          reason={
            !destination
              ? "This connection is no longer available in this scope. Return to the connection list and choose its current destination."
              : "A Control Plane administrator manages this connection's credential assignment."
          }
        />
      );
    if (
      data &&
      !executionUnavailable &&
      editorLocation.action === "edit" &&
      !data.credentials.some((row) => row.id === editorLocation.id)
    ) {
      return (
        <SettingsEditorUnavailable
          backLabel={inventoryLabel}
          onBack={closeEditor}
        />
      );
    }
    return (
      <SettingsEditorPage
        title={
          editorLocation.action === "edit"
            ? "Rotate credential"
            : "Add credential"
        }
        description={description}
        backLabel={inventoryLabel}
        onBack={closeEditor}
        busy={saving}
      >
        <LoadState {...resource} retry={resource.refresh} />
        {error ? <p role="alert">{error}</p> : null}
        {executionUnavailable ? (
          <p role="status">{executionReason}</p>
        ) : !canEdit ? (
          <p role="alert">
            You do not have permission to change credentials in this scope.
          </p>
        ) : readyRoute === routeKey && data ? (
          editorContent
        ) : null}
      </SettingsEditorPage>
    );
  }
  return (
    <section
      aria-label={
        globalScope
          ? "Control Plane secrets"
          : projectScope
            ? "Project credentials"
            : "Personal credentials"
      }
    >
      <LoadState {...resource} retry={resource.refresh} />
      {error ? <p role="alert">{error}</p> : null}
      {notice ? <p role="status">{notice}</p> : null}
      <ToastRegion
        toasts={toasts}
        onDismiss={dismissToast}
        placement="bottom"
      />
      {executionUnavailable ? <p role="status">{executionReason}</p> : null}
      <CatalogInventory
        key={scope}
        kind="credential"
        title={globalScope || projectScope ? "Secrets" : "Credentials"}
        description={description}
        headingLevel={2}
        summary={`${data?.credentials.length ?? 0} saved credentials · ${data?.connections.length ?? 0} available connections`}
        busy={busy}
        editing={Boolean(editor)}
        onAdd={() => edit()}
        addInToolbar
        toolbarActions={
          <>
            <Button
              disabled={
                busy ||
                Boolean(editor) ||
                !data?.credentials.length ||
                !data?.connections.length
              }
              onClick={() => {
                reviewed.current = data;
                setId("");
                setTarget("");
                setEditor("attach");
              }}
            >
              Attach existing credential
            </Button>
            <Button
              className="ui-icon-button"
              aria-label="Reload credentials"
              title="Reload credentials"
              disabled={
                saving || resource.loading || (Boolean(editor) && !mustReload)
              }
              onClick={() => {
                setEditor(null);
                setDirty(false);
                setMustReload(false);
                reviewed.current = null;
                resource.refresh();
              }}
            >
              <IconRefresh size={16} aria-hidden="true" />
            </Button>
          </>
        }
        emptyDescription="Save an API key or token now. You can configure its connection later."
        rows={(data?.credentials ?? []).map((row) => ({
          id: row.id,
          label: row.label,
          name: row.label,
          description: kinds[row.kind],
          searchText: `${row.label} ${kinds[row.kind]} ${row.used_by.map((usage) => usage.label).join(" ")}`,
          connection: (
            <span className="credential-status tone-success-text">
              <IconLock size={15} aria-hidden="true" />
              <span>Stored securely</span>
            </span>
          ),
          usage: row.used_by.length
            ? row.used_by.map((usage) => usage.label).join(", ")
            : globalScope
              ? "No assignments in selected project"
              : "Not attached",
          actionLabel: "Rotate",
          actionAriaLabel: `Rotate credential ${row.label}`,
          mutationDisabled: Boolean(editor),
          onEdit: () => edit(row),
          onDelete: () => {
            reviewed.current = data;
            setId(row.id);
            setEditor("remove");
            setError("");
          },
          details: (
            <>
              <div>
                <h4>Credential details</h4>
                <dl>
                  <dt>Type</dt>
                  <dd>{kinds[row.kind]}</dd>
                  <dt>Access</dt>
                  <dd>
                    {globalScope
                      ? "Instance secret with explicit project assignments"
                      : projectScope
                        ? "Shared project credential"
                        : "Your personal credential"}
                  </dd>
                </dl>
                <p className="field-help">
                  Encrypted in server storage. Saved values are never returned
                  to the browser.
                </p>
              </div>
              <div>
                <h4>Used by</h4>
                {row.used_by.length ? (
                  row.used_by.map((usage) => (
                    <p key={destinationKey(usage)}>
                      {usage.label}
                      <br />
                      <span className="field-help">{usage.endpoint}</span>
                      <br />
                      <Button
                        disabled={busy || Boolean(editor)}
                        onClick={() => void attach(null, usage).catch(() => {})}
                      >
                        Detach from {usage.label}
                      </Button>
                    </p>
                  ))
                ) : (
                  <p>
                    {globalScope
                      ? "No assignments in the selected project. This secret may be assigned in other projects."
                      : "Not attached. No connection can use this credential."}
                  </p>
                )}
              </div>
            </>
          ),
        }))}
      >
        {editorContent}
      </CatalogInventory>
      {assignmentContext ? (
        <section aria-label="Connection assignments">
          <header className="catalog-heading">
            <h2>Assignments</h2>
          </header>
          {assignmentContext}
        </section>
      ) : null}
    </section>
  );
}
