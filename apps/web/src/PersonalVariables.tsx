import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { projectPath, request } from "./api";
import { validVariableName } from "./chat-settings";
import { errorMessage, useResource } from "./resources";
import { LoadState } from "./Home";

export function PersonalVariables({
  project,
  identity,
  canEdit,
}: {
  project: string;
  identity: string;
  canEdit: boolean;
}) {
  const path = `${projectPath(project)}/chat/variables/personal`;
  const resource = useResource<{
    revision: number;
    variables: Record<string, string>;
  }>(project ? path : null, 0, identity);
  const [editor, setEditor] = useState<{
    original: string;
    name: string;
    value: string;
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const [waitingRevision, setWaitingRevision] = useState(0);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const scope = JSON.stringify([project, identity, canEdit]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  useEffect(() => {
    setEditor(null);
    setSaving(false);
    setWaitingRevision(0);
    setError("");
    setNotice("");
    return () => {
      currentScope.current = "";
    };
  }, [scope]);
  const busy =
    saving ||
    resource.loading ||
    Boolean(
      waitingRevision &&
      (!resource.data || resource.data.revision < waitingRevision),
    );
  async function save(variables: Record<string, string>) {
    if (!resource.data || busy || !canEdit) return;
    const submitted = scope;
    setSaving(true);
    setError("");
    setNotice("");
    try {
      const result = await request<{ revision: number }>(
        path,
        { revision: resource.data.revision, variables },
        undefined,
        "PUT",
      );
      if (currentScope.current !== submitted) return;
      setWaitingRevision(result.revision);
      setEditor(null);
      setNotice("Your project variables were saved.");
      resource.refresh();
    } catch (cause) {
      if (currentScope.current === submitted) {
        setError(`${errorMessage(cause)} Reload variables before retrying.`);
        resource.refresh();
      }
    } finally {
      if (currentScope.current === submitted) setSaving(false);
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
            onClick={() => setEditor({ original: row.name, ...row })}
          >
            Edit {row.name}
          </Button>
          <Button
            variant="danger"
            disabled={busy || !canEdit || Boolean(editor)}
            onClick={() => {
              const variables = { ...values };
              delete variables[row.name];
              void save(variables);
            }}
          >
            Remove
          </Button>
        </div>
      ),
    },
  ];
  return (
    <section aria-label="Personal project variables">
      <header className="catalog-heading">
        <div>
          <h2>Your variables</h2>
          <p>
            Private to your account in the selected project. These are nonsecret
            configuration values; use Secrets for API keys and tokens. Variables
            are not injected into runtime environments.
          </p>
        </div>
        <Button
          disabled={busy || !canEdit || Boolean(editor) || !resource.data}
          onClick={() => setEditor({ original: "", name: "", value: "" })}
        >
          Add variable
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? <p role="alert">{error}</p> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {editor ? (
        <form
          className="chat-settings-editor"
          onSubmit={(event) => {
            event.preventDefault();
            if (
              !validVariableName(editor.name) ||
              new TextEncoder().encode(editor.value).length > 4096 ||
              editor.value.includes("\0")
            ) {
              setError(
                "Use 1–128 letters, digits, or underscores for the name and at most 4,096 UTF-8 bytes for the value.",
              );
              return;
            }
            if (!editor.original && Object.hasOwn(values, editor.name)) {
              setError("That variable already exists.");
              return;
            }
            void save({ ...values, [editor.name]: editor.value });
          }}
        >
          <h3>{editor.original ? "Edit variable" : "Add variable"}</h3>
          <label>
            Name
            <TextInput
              aria-label="Variable name"
              required
              value={editor.name}
              maxLength={128}
              disabled={busy || Boolean(editor.original)}
              onChange={(event) =>
                setEditor({ ...editor, name: event.target.value })
              }
            />
          </label>
          <label>
            Value
            <TextInput
              aria-label="Variable value"
              value={editor.value}
              maxLength={4096}
              disabled={busy}
              onChange={(event) =>
                setEditor({ ...editor, value: event.target.value })
              }
            />
          </label>
          <div className="chat-settings-actions">
            <Button type="submit" variant="primary" disabled={busy}>
              Save variable
            </Button>
            <Button disabled={busy} onClick={() => setEditor(null)}>
              Cancel
            </Button>
          </div>
        </form>
      ) : null}
      <DataTable
        data={rows}
        columns={columns}
        getRowId={(row) => row.name}
        label="Your project variables"
        search={{ columnId: "name", label: "Search your variables" }}
        empty="No personal variables configured."
      />
    </section>
  );
}
