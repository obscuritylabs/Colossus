import type { CredentialTarget } from "./credential-types";
import { useEffect, useRef, useState } from "react";
import {
  Button,
  CatalogInventory,
  DropdownSelect,
  TextInput,
} from "@colossus/ui";
import { CredentialEditor } from "@colossus/ui/components/CredentialEditor";
import { CredentialForm } from "@colossus/ui/components/CredentialForm";
import { IconLock } from "@tabler/icons-react";
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
}: {
  path: string;
  identity: string;
  canEdit: boolean;
  projectScope?: boolean;
  globalScope?: boolean;
  resourcePath?: string;
  onSaved: () => void;
  initialConnection?: CredentialTarget | undefined;
}) {
  const resource = useResource<CredentialSnapshot>(resourcePath, 0, identity);
  const [editor, setEditor] = useState<"secret" | "attach" | "remove" | null>(
    null,
  );
  const [id, setId] = useState("");
  const [label, setLabel] = useState("");
  const [kind, setKind] = useState<Kind>("api_key");
  const [target, setTarget] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [saving, setSaving] = useState(false);
  const [waitingRevision, setWaitingRevision] = useState(0);
  const scope = JSON.stringify([path, resourcePath, identity, canEdit]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const opened = useRef("");
  useEffect(() => {
    setEditor(null);
    setId("");
    setLabel("");
    setTarget("");
    setError("");
    setNotice("");
    setSaving(false);
    setWaitingRevision(0);
    opened.current = "";
  }, [scope]);
  useEffect(
    () => () => {
      currentScope.current = "";
    },
    [],
  );
  const data = resource.data;
  const busy =
    saving ||
    !canEdit ||
    resource.loading ||
    Boolean(waitingRevision && (!data || data.revision < waitingRevision));
  const credential = data?.credentials.find((row) => row.id === id);
  const connection = data?.connections.find(
    (row) => destinationKey(row) === target,
  );
  function edit(value?: Credential, destination?: Destination) {
    setId(value?.id ?? "");
    setLabel(value?.label ?? "");
    setKind(value?.kind ?? "api_key");
    setTarget(destination ? destinationKey(destination) : "");
    setEditor("secret");
    setError("");
    setNotice("");
  }
  useEffect(() => {
    const key = JSON.stringify([scope, initialConnection]);
    if (!initialConnection || !data || !canEdit || opened.current === key)
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
    if (destination && !existing && data.credentials.length) {
      setId("");
      setTarget(destinationKey(destination));
      setEditor("attach");
    } else edit(existing, destination);
  }, [scope, initialConnection, data, canEdit, globalScope]);
  async function mutate(url: string, body: object, method?: "DELETE") {
    const submitted = scope;
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
      setWaitingRevision(result.revision);
      setEditor(null);
      setNotice("Credential changes saved.");
      resource.refresh();
      onSaved();
    } catch (e) {
      if (currentScope.current === submitted) {
        setError(`${errorMessage(e)} Reload credentials before retrying.`);
        resource.refresh();
      }
      throw e;
    } finally {
      if (currentScope.current === submitted) setSaving(false);
    }
  }
  async function attach(value: string | null, destination: Destination) {
    if (!data) return;
    await mutate(
      `${path}/attachments/${globalScope ? `${encodeURIComponent(destination.project_id ?? "")}/` : ""}${destination.kind}/${encodeURIComponent(destination.id)}`,
      {
        revision: data.revision,
        credential_id: value,
        credential_binding: destination.credential_binding,
      },
    );
  }
  const description = globalScope
    ? "Instance secrets are write-only. A Control Plane administrator must explicitly assign a secret to a shared connection in a selected project before it can be used."
    : projectScope
      ? "Shared within this project. Only explicitly attached connections may use a saved credential."
      : "Private to your account in this project. Save a credential now and attach it to a provider, search service, or MCP server later.";
  const destinationOptions = (
    <>
      <option value="">
        {editor === "attach" ? "Choose a connection" : "Save without attaching"}
      </option>
      {(data?.connections ?? []).map((row) => (
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
      <p className="field-help">{description}</p>
      <CatalogInventory
        key={scope}
        kind="credential"
        headingLevel={2}
        summary={`${data?.credentials.length ?? 0} saved credentials · ${data?.connections.length ?? 0} available connections`}
        busy={busy}
        editing={Boolean(editor)}
        onAdd={() => edit()}
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
        {editor === "secret" ? (
          <CredentialEditor
            title={id ? "Rotate credential" : "Add credential"}
            description={description}
            busy={busy}
            onCancel={() => setEditor(null)}
          >
            <label>
              <span>Display label</span>
              <TextInput
                value={label}
                maxLength={160}
                disabled={busy}
                onChange={(event) => setLabel(event.target.value)}
                placeholder="For example, GitHub workspace token"
              />
            </label>
            <label>
              <span>Credential type</span>
              <DropdownSelect
                aria-label="Credential type"
                value={kind}
                disabled={busy}
                onChange={(event) => setKind(event.target.value as Kind)}
              >
                {Object.entries(kinds).map(([value, text]) => (
                  <option key={value} value={value}>
                    {text}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            <label>
              <span>Attach to connection</span>
              <DropdownSelect
                aria-label="Credential connection"
                value={target}
                disabled={busy}
                onChange={(event) => setTarget(event.target.value)}
              >
                {destinationOptions}
              </DropdownSelect>
            </label>
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
            <CredentialForm
              key={JSON.stringify([scope, id, target, data?.revision])}
              scope={JSON.stringify([scope, id, target, data?.revision])}
              label="API key or token"
              action={id ? "Save rotated credential" : "Save credential"}
              endpoint={connection?.endpoint ?? "Not attached to a connection"}
              disabled={busy || !label.trim()}
              description={
                connection
                  ? "Saving attaches this credential to the destination shown above."
                  : "Saving alone gives agents no access to this credential."
              }
              onSave={async (secret) =>
                mutate(id ? `${path}/${encodeURIComponent(id)}` : path, {
                  revision: data!.revision,
                  label: label.trim(),
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
          </CredentialEditor>
        ) : null}
        {editor === "attach" ? (
          <CredentialEditor
            title="Attach credential"
            description="Choose a saved credential and review the destination that will receive it."
            busy={busy}
            onCancel={() => setEditor(null)}
            submitLabel="Attach credential"
            submitDisabled={!id || !connection}
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
                onChange={(event) => setId(event.target.value)}
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
                onChange={(event) => setTarget(event.target.value)}
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
            busy={busy}
            onCancel={() => setEditor(null)}
            submitLabel="Remove credential"
            onSubmit={() =>
              void mutate(
                `${path}/${encodeURIComponent(id)}`,
                { revision: data!.revision },
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
      </CatalogInventory>
      <div className="chat-settings-actions">
        <Button
          disabled={
            busy ||
            Boolean(editor) ||
            !data?.credentials.length ||
            !data?.connections.length
          }
          onClick={() => {
            setId("");
            setTarget("");
            setEditor("attach");
          }}
        >
          Attach existing credential
        </Button>
        <Button disabled={saving} onClick={resource.refresh}>
          Reload credentials
        </Button>
      </div>
    </section>
  );
}
