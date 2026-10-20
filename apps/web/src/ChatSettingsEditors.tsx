import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import { ChatProviderSettings } from "./ChatProviderSettings";
import { ChatModelSettings } from "./ChatModelSettings";
import {
  tokens,
  validProfile,
  type Authentication,
  type ChatSettingsWrite,
  type ChatSettingsSnapshot,
  type McpServer,
} from "./chat-settings";
export type SettingsEditor = { kind: "provider" | "model" | "mcp"; id: string };
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="chat-setting-field">
      <span>{label}</span>
      {children}
    </label>
  );
}
function AuthField({
  value,
  onChange,
}: {
  value: Authentication;
  onChange: (value: Authentication) => void;
}) {
  return (
    <Field label="Authentication">
      <DropdownSelect
        aria-label="Authentication"
        value={value}
        onChange={(event) => onChange(event.target.value as Authentication)}
      >
        <option value="personal">Personal account (each user)</option>
        <option value="shared">Shared project credential</option>
        <option value="none">No authentication</option>
      </DropdownSelect>
    </Field>
  );
}
interface EditorProps {
  editor: SettingsEditor;
  draft: ChatSettingsWrite;
  project: string;
  saved: ChatSettingsSnapshot;
  onApply: (value: ChatSettingsWrite) => void;
  onCancel: () => void;
  canManageCredentials: boolean;
  onBusyChange?: ((busy: boolean) => void) | undefined;
  onCredentials?: (() => void) | undefined;
  onPersonalCredentials?: (() => void) | undefined;
}
export function ChatSettingsEditor(props: EditorProps) {
  if (props.editor.kind === "provider")
    return <ChatProviderSettings {...props} id={props.editor.id} />;
  if (props.editor.kind === "model")
    return <ChatModelSettings {...props} id={props.editor.id} />;
  return <McpEditor {...props} />;
}
function McpEditor({ editor, draft, onApply, onCancel }: EditorProps) {
  const originalMcp = draft.mcp.servers[editor.id];
  const [id, setId] = useState(editor.id),
    [url, setUrl] = useState(originalMcp?.url ?? "");
  const [auth, setAuth] = useState<Authentication>(
    draft.mcp_authentication[editor.id] ?? "personal",
  );
  const [tools, setTools] = useState(
      originalMcp?.allowedTools.join(", ") ?? "",
    ),
    [oauth, setOauth] = useState(Boolean(originalMcp?.oauth));
  const [client, setClient] = useState(originalMcp?.oauth?.clientId ?? ""),
    [scopes, setScopes] = useState(originalMcp?.oauth?.scopes.join(", ") ?? "");
  const [error, setError] = useState("");
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, []);
  function submit() {
    if (!validProfile(id)) {
      setError("Use a valid profile ID.");
      return;
    }
    if (!editor.id && Object.hasOwn(draft.mcp.servers, id)) {
      setError("That profile ID already exists.");
      return;
    }
    const next = structuredClone(draft);
    const allowed = tokens(tools);
    if (!allowed.length || (allowed.includes("*") && allowed.length !== 1)) {
      setError(
        "Enter exact allowed tool names or patterns. A wildcard must be the only entry.",
      );
      return;
    }
    if (oauth && auth !== "personal") {
      setError(
        "OAuth connections use each user's personal sign-in. Choose Personal account, or use a static credential for shared access.",
      );
      return;
    }
    const value: McpServer = {
      ...originalMcp,
      transport: "streamable_http",
      url: url.trim(),
      allowedTools: allowed,
      oauth: oauth
        ? {
            ...originalMcp?.oauth,
            clientId: client.trim(),
            scopes: tokens(scopes),
            callbackPort: originalMcp?.oauth?.callbackPort ?? 8787,
          }
        : null,
    };
    if (oauth || auth === "none") value.credentialHeaders = {};
    else if (!Object.keys(value.credentialHeaders ?? {}).length)
      value.credentialHeaders = {
        Authorization: { scheme: "Bearer", reference: "host:managed" },
      };
    next.mcp.servers[id] = value;
    next.mcp_authentication[id] = auth;
    onApply(next);
  }
  return (
    <form
      className="chat-settings-editor"
      aria-label="Edit mcp"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <h3 ref={heading} tabIndex={-1}>
        {editor.id ? "Edit" : "Add"} MCP connection
      </h3>
      {error ? <p role="alert">{error}</p> : null}
      <div className="chat-settings-fields">
        <Field label="Profile ID">
          <TextInput
            aria-label="Profile ID"
            value={id}
            disabled={Boolean(editor.id)}
            required
            onChange={(event) => setId(event.target.value)}
          />
        </Field>
        <Field label="MCP server URL">
          <TextInput
            aria-label="MCP server URL"
            type="url"
            value={url}
            required
            maxLength={2048}
            onChange={(event) => setUrl(event.target.value)}
          />
        </Field>
        <Field label="Allowed tools">
          <TextInput
            aria-label="Allowed tools"
            value={tools}
            required
            placeholder="search, read_document"
            onChange={(event) => setTools(event.target.value)}
          />
        </Field>
        <AuthField value={auth} onChange={setAuth} />
        <Field label="Sign-in method">
          <DropdownSelect
            aria-label="Sign-in method"
            value={oauth ? "oauth" : "static"}
            onChange={(event) => setOauth(event.target.value === "oauth")}
          >
            <option value="static">
              Static credential or no authentication
            </option>
            <option value="oauth">OAuth sign-in</option>
          </DropdownSelect>
        </Field>
        {oauth ? (
          <>
            <Field label="OAuth client ID">
              <TextInput
                aria-label="OAuth client ID"
                value={client}
                required
                onChange={(event) => setClient(event.target.value)}
              />
            </Field>
            <Field label="OAuth scopes">
              <TextInput
                aria-label="OAuth scopes"
                value={scopes}
                placeholder="read, search"
                onChange={(event) => setScopes(event.target.value)}
              />
            </Field>
          </>
        ) : null}
      </div>
      <div className="chat-settings-actions">
        <Button type="submit" variant="primary">
          {editor.id ? "Save changes" : "Add MCP connection"}
        </Button>
        <Button onClick={onCancel}>Cancel</Button>
      </div>
    </form>
  );
}
