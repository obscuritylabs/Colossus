import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { projectPath, request } from "./api";
import { validVariableName } from "./chat-settings";
import { errorMessage, useResource } from "./resources";
import { LoadState } from "./Home";
import type { CredentialExecution } from "./credential-types";
import {
  SettingsEditorPage,
  SettingsEditorUnavailable,
  type SettingsEditorLocation,
} from "./settings-editor";

interface VariableSnapshot {
  execution?: CredentialExecution;
  revision: number;
  variables: Record<string, string>;
}
interface VariableDraft {
  original: string;
  name: string;
  value: string;
  initialValue: string;
  snapshot: VariableSnapshot;
}

export function PersonalVariables({
  project,
  identity,
  canEdit,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
  onSaved,
}: {
  project: string;
  identity: string;
  canEdit: boolean;
  editorLocation?: SettingsEditorLocation | undefined;
  onOpenEditor?: ((location: SettingsEditorLocation) => void) | undefined;
  onCloseEditor?: (() => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  onSaved?: (() => void) | undefined;
}) {
  const path = `${projectPath(project)}/chat/variables/personal`;
  const resource = useResource<VariableSnapshot>(
    project ? path : null,
    0,
    identity,
  );
  const [editor, setEditor] = useState<VariableDraft | null>(null);
  const [saving, setSaving] = useState(false);
  const [waitingRevision, setWaitingRevision] = useState(0);
  const [mustReload, setMustReload] = useState(false);
  const [readyRoute, setReadyRoute] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const scope = JSON.stringify([project, identity, canEdit]);
  const currentScope = useRef(scope);
  const pending = useRef<object | null>(null);
  currentScope.current = scope;
  const routeKey = JSON.stringify([
    scope,
    editorLocation?.action,
    editorLocation?.id,
  ]);
  const view = JSON.stringify([scope, editorLocation]);
  const operationScope = useRef({ view });
  if (operationScope.current.view !== view) operationScope.current = { view };
  const dirty = Boolean(
    editor &&
    (editor.original
      ? editor.value !== editor.initialValue
      : editor.name || editor.value),
  );
  useEffect(() => {
    onDirtyChange?.(dirty || saving);
    return () => onDirtyChange?.(false);
  }, [dirty, saving, onDirtyChange]);
  useEffect(() => {
    currentScope.current = scope;
    setEditor(null);
    setSaving(false);
    setWaitingRevision(0);
    setMustReload(false);
    setReadyRoute("");
    setError("");
    setNotice("");
    return () => {
      currentScope.current = "";
    };
  }, [scope]);
  useEffect(() => {
    if (!editorLocation) {
      if (readyRoute) {
        setEditor(null);
        setReadyRoute("");
      }
      return;
    }
    const snapshot = resource.data;
    if (!snapshot || readyRoute === routeKey) return;
    const original =
      editorLocation.action === "edit" ? (editorLocation.id ?? "") : "";
    const value = Object.hasOwn(snapshot.variables, original)
      ? snapshot.variables[original]!
      : "";
    setEditor({
      original,
      name: original,
      value,
      initialValue: value,
      snapshot,
    });
    setError("");
    setNotice("");
    setMustReload(false);
    setReadyRoute(routeKey);
    setSaving(false);
  }, [editorLocation, readyRoute, resource.data, routeKey]);
  const executionUnavailable =
    resource.data?.execution?.settings_editable === false;
  const executionReason =
    resource.data?.execution?.reason ||
    "A deployment operator must provision this project's hosted runtime before personal variables can be configured.";
  const busy =
    executionUnavailable ||
    saving ||
    mustReload ||
    resource.loading ||
    Boolean(
      waitingRevision &&
      (!resource.data || resource.data.revision < waitingRevision),
    );
  function closeEditor() {
    if (saving) return;
    if (editorLocation && onCloseEditor) onCloseEditor();
    else setEditor(null);
  }
  function openEditor(name?: string) {
    if (executionUnavailable || !canEdit) return;
    if (onOpenEditor) {
      onOpenEditor(
        name === undefined ? { action: "new" } : { action: "edit", id: name },
      );
      return;
    }
    const snapshot = resource.data;
    if (!snapshot) return;
    const value = name === undefined ? "" : (snapshot.variables[name] ?? "");
    setEditor({
      original: name ?? "",
      name: name ?? "",
      value,
      initialValue: value,
      snapshot,
    });
    setError("");
    setNotice("");
  }
  async function save(variables: Record<string, string>, revision: number) {
    if (
      !resource.data ||
      busy ||
      !canEdit ||
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
        path,
        { revision, variables },
        undefined,
        "PUT",
      );
      if (currentScope.current !== submitted) return;
      resource.refresh();
      if (operationScope.current !== submittedOperation) return;
      setWaitingRevision(result.revision);
      setEditor(null);
      onDirtyChange?.(false);
      onSaved?.();
      if (editorLocation) onCloseEditor?.();
      setNotice("Your project variables were saved.");
    } catch (cause) {
      if (
        currentScope.current === submitted &&
        operationScope.current === submittedOperation
      ) {
        setMustReload(true);
        setError(
          `${errorMessage(cause)} Return to variables and reload them before retrying.`,
        );
        resource.refresh();
      }
    } finally {
      if (pending.current === submittedOperation) pending.current = null;
      if (
        currentScope.current === submitted &&
        operationScope.current === submittedOperation
      )
        setSaving(false);
    }
  }
  if (!project)
    return <p>Choose a project to manage your personal variables.</p>;
  const values = resource.data?.variables ?? {};
  const rows = Object.entries(values).map(([name, value]) => ({ name, value }));
  const columns: DataTableColumn<(typeof rows)[number]>[] = [
    { id: "name", label: "Name", value: (row) => row.name },
    { id: "value", label: "Value", value: (row) => row.value },
    {
      id: "actions",
      label: "Actions",
      value: () => "",
      cell: (row) => (
        <div className="chat-settings-row-actions">
          <Button
            disabled={busy || !canEdit || Boolean(editor)}
            onClick={() => openEditor(row.name)}
          >
            Edit {row.name}
          </Button>
          <Button
            variant="danger"
            disabled={busy || !canEdit || Boolean(editor)}
            onClick={() => {
              const variables = { ...values };
              delete variables[row.name];
              void save(variables, resource.data!.revision);
            }}
          >
            Remove
          </Button>
        </div>
      ),
    },
  ];
  const description =
    "Private to your account in the selected project. These are nonsecret configuration values; use Credentials for API keys and tokens. Variables are not injected into runtime environments.";
  const editorForm = editor ? (
    <form
      className="chat-settings-editor management-form"
      aria-label={
        editor.original ? "Edit personal variable" : "Add personal variable"
      }
      onSubmit={(event) => {
        event.preventDefault();
        if (!canEdit || busy) return;
        if (
          !validVariableName(editor.name) ||
          new TextEncoder().encode(editor.value).length > 4096 ||
          editor.value.includes("\0")
        ) {
          setError(
            "Use 1–128 letters, digits, or underscores for the name and at most 4,096 UTF-8 bytes for the value. Values cannot contain null characters.",
          );
          return;
        }
        if (
          !editor.original &&
          Object.hasOwn(editor.snapshot.variables, editor.name)
        ) {
          setError("That variable already exists.");
          return;
        }
        if (
          !editor.original &&
          Object.keys(editor.snapshot.variables).length >= 256
        ) {
          setError(
            "A personal project store supports up to 256 variables. Remove a variable before adding another.",
          );
          return;
        }
        void save(
          { ...editor.snapshot.variables, [editor.name]: editor.value },
          editor.snapshot.revision,
        );
      }}
    >
      {!editorLocation ? (
        <h3>{editor.original ? "Edit variable" : "Add variable"}</h3>
      ) : null}
      <label>
        Name
        <TextInput
          aria-label="Variable name"
          required
          value={editor.name}
          maxLength={128}
          readOnly={Boolean(editor.original)}
          disabled={busy || !canEdit}
          onChange={(event) =>
            setEditor({ ...editor, name: event.target.value })
          }
        />
        <small>
          {editor.original
            ? "Names are fixed after creation. Create another variable to use a different name."
            : "Use letters, digits, and underscores. Names are case-sensitive."}
        </small>
      </label>
      <label>
        Value
        <textarea
          className="ui-input"
          aria-label="Variable value"
          rows={4}
          value={editor.value}
          maxLength={4096}
          disabled={busy || !canEdit}
          onChange={(event) =>
            setEditor({ ...editor, value: event.target.value })
          }
        />
        <small>
          {new TextEncoder().encode(editor.value).length.toLocaleString()} /
          4,096 UTF-8 bytes
        </small>
      </label>
      <div className="chat-settings-actions">
        <Button
          type="submit"
          variant="primary"
          disabled={busy || !canEdit || !editor.name || !dirty}
        >
          Save variable
        </Button>
        <Button disabled={saving} onClick={closeEditor}>
          Cancel
        </Button>
      </div>
    </form>
  ) : null;
  if (editorLocation) {
    if (
      resource.data &&
      !executionUnavailable &&
      editorLocation.action === "edit" &&
      !Object.hasOwn(values, editorLocation.id ?? "")
    ) {
      return (
        <SettingsEditorUnavailable backLabel="variables" onBack={closeEditor} />
      );
    }
    return (
      <SettingsEditorPage
        title={
          editorLocation.action === "edit" ? "Edit variable" : "Add variable"
        }
        description={description}
        backLabel="variables"
        onBack={closeEditor}
        busy={saving}
      >
        <LoadState {...resource} retry={resource.refresh} />
        {error ? <p role="alert">{error}</p> : null}
        {executionUnavailable ? (
          <p role="status">{executionReason}</p>
        ) : !canEdit ? (
          <p role="alert">
            You do not have permission to change variables in this project.
          </p>
        ) : readyRoute === routeKey ? (
          editorForm
        ) : null}
      </SettingsEditorPage>
    );
  }
  return (
    <section aria-label="Personal project variables">
      <header className="catalog-heading">
        <div>
          <h2>Your variables</h2>
          <p>{description}</p>
        </div>
        <Button
          disabled={
            busy ||
            !canEdit ||
            Boolean(editor) ||
            !resource.data ||
            rows.length >= 256
          }
          onClick={() => openEditor()}
        >
          Add variable
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? <p role="alert">{error}</p> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {executionUnavailable ? <p role="status">{executionReason}</p> : null}
      {editorForm}
      <DataTable
        data={rows}
        columns={columns}
        getRowId={(row) => row.name}
        label="Your project variables"
        search={{ columnId: "name", label: "Search your variables" }}
        empty="No personal variables configured."
      />
      <div className="chat-settings-actions">
        <Button
          disabled={saving}
          onClick={() => {
            setEditor(null);
            setMustReload(false);
            resource.refresh();
          }}
        >
          Reload variables
        </Button>
      </div>
    </section>
  );
}
