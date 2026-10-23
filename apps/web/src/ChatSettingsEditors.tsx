import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import { IconPlug } from "@tabler/icons-react";
import { ProviderSetupSection } from "@colossus/ui/components/ProviderSetupSection";
import "@colossus/ui/styles/provider-settings-editor.css";
import { ChatProviderSettings } from "./ChatProviderSettings";
import { ChatModelSettings } from "./ChatModelSettings";
import type { CredentialSetup, CredentialTarget } from "./credential-types";
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
        <option value="shared">Shared credential (explicit assignment)</option>
        <option value="none">No authentication</option>
      </DropdownSelect>
    </Field>
  );
}
interface EditorProps {
  editor: SettingsEditor;
  draft: ChatSettingsWrite;
  project: string;
  settingsPath?: string;
  instanceScope?: boolean;
  saved: ChatSettingsSnapshot;
  onApply: (value: ChatSettingsWrite, setup?: CredentialSetup) => void;
  onCancel: () => void;
  canManageCredentials: boolean;
  busy?: boolean;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  onBusyChange?: ((busy: boolean) => void) | undefined;
  onCredentials?: ((target?: CredentialTarget) => void) | undefined;
  onPersonalCredentials?: ((target?: CredentialTarget) => void) | undefined;
}
export function ChatSettingsEditor(props: EditorProps) {
  if (props.editor.kind === "provider")
    return <ChatProviderSettings {...props} id={props.editor.id} />;
  if (props.editor.kind === "model")
    return (
      <ChatModelSettings
        {...props}
        id={props.editor.id}
        catalogEnabled={!props.instanceScope}
      />
    );
  return <McpEditor {...props} />;
}
function McpEditor({
  editor,
  draft,
  onApply,
  onCancel,
  onCredentials,
  onPersonalCredentials,
  busy = false,
  onDirtyChange,
}: EditorProps) {
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
  const helpId = useId();
  const formValue = JSON.stringify([
    id,
    url,
    auth,
    tools,
    oauth,
    client,
    scopes,
  ]);
  const openingValue = useRef(formValue);
  useEffect(() => {
    onDirtyChange?.(formValue !== openingValue.current);
  }, [formValue, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    heading.current?.focus();
  }, []);
  function submit(continueToCredential = false) {
    if (busy) return;
    if (!validProfile(id)) {
      setError(
        "Use a nonreserved Profile ID of 1–128 letters, digits, hyphens, or underscores, starting with a letter or digit.",
      );
      return;
    }
    if (!editor.id && Object.hasOwn(draft.mcp.servers, id)) {
      setError("That profile ID already exists.");
      return;
    }
    try {
      const endpoint = new URL(url.trim());
      if (
        !["http:", "https:"].includes(endpoint.protocol) ||
        endpoint.username ||
        endpoint.password ||
        !endpoint.hostname ||
        url.includes("?") ||
        url.includes("#")
      )
        throw new Error();
      const loopback =
        endpoint.hostname === "localhost" ||
        endpoint.hostname === "[::1]" ||
        /^127(?:\.\d{1,3}){3}$/u.test(endpoint.hostname);
      if (endpoint.protocol === "http:" && !loopback) {
        setError(
          "Use an HTTPS MCP endpoint, or HTTP for a loopback connection on the Control Plane server.",
        );
        return;
      }
    } catch {
      setError(
        "Enter an HTTP or HTTPS MCP endpoint without embedded credentials, a query, or a fragment.",
      );
      return;
    }
    const allowed = tools.split(/[\s,]+/u).filter(Boolean);
    if (
      !allowed.length ||
      allowed.length > 1024 ||
      allowed.some(
        (tool) =>
          !/^[A-Za-z0-9._*-]{1,128}$/u.test(tool) || tool.includes("**"),
      )
    ) {
      setError(
        "Enter 1–1,024 exact tool names or * patterns using 1–128 ASCII letters, digits, dots, underscores, or hyphens. Regex, control characters, and consecutive stars are unsupported.",
      );
      return;
    }
    if (allowed.includes("*") && allowed.length !== 1) {
      setError(
        "Use * alone to allow all tools, or list exact names and narrower * patterns.",
      );
      return;
    }
    if (new Set(allowed).size !== allowed.length) {
      setError("Use each tool selector only once.");
      return;
    }
    if (oauth && !client.trim()) {
      setError("Enter the registered OAuth client ID.");
      return;
    }
    const oauthClient = client.trim();
    const oauthScopes = tokens(scopes);
    const byteLength = (value: string) =>
      new TextEncoder().encode(value).length;
    if (
      oauth &&
      (byteLength(oauthClient) > 1024 ||
        /[\u0000-\u001f\u007f]/u.test(oauthClient))
    ) {
      setError(
        "Use an OAuth client ID of at most 1,024 UTF-8 bytes without control characters.",
      );
      return;
    }
    if (
      oauth &&
      (oauthScopes.length > 32 ||
        oauthScopes.some(
          (scope) =>
            byteLength(scope) > 256 || /[\u0000-\u0020\u007f]/u.test(scope),
        ))
    ) {
      setError(
        "Use at most 32 OAuth scopes, each at most 256 UTF-8 bytes without control characters or spaces.",
      );
      return;
    }
    if (oauth && auth !== "personal") {
      setError(
        "OAuth connections use each user's personal sign-in. Choose Personal account, or use a static credential for shared access.",
      );
      return;
    }
    const next = structuredClone(draft);
    const value: McpServer = {
      ...originalMcp,
      transport: "streamable_http",
      url: url.trim(),
      allowedTools: allowed,
      oauth: oauth
        ? {
            ...originalMcp?.oauth,
            clientId: oauthClient,
            scopes: oauthScopes,
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
    onApply(
      next,
      continueToCredential && auth !== "none"
        ? { target: { kind: "mcp", id }, authentication: auth }
        : undefined,
    );
  }
  return (
    <form
      className="chat-settings-editor shared-provider-settings-editor mcp-settings-editor"
      aria-label="MCP connection settings"
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <div className="provider-editor-heading">
        <span className="resource-icon">
          <IconPlug size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 ref={heading} tabIndex={-1}>
            {editor.id ? "Edit" : "Add"} MCP connection
          </h4>
          <p>Connect an MCP server and choose which tools it exposes.</p>
        </div>
      </div>
      {error ? <p role="alert">{error}</p> : null}
      <fieldset disabled={busy} className="mcp-settings-fieldset">
        <ProviderSetupSection
          title="Connection"
          description="Name this connection and enter the server address."
        >
          <div className="chat-settings-fields">
            <Field label="Profile ID">
              <TextInput
                aria-label="Profile ID"
                value={id}
                disabled={Boolean(editor.id)}
                required
                maxLength={128}
                spellCheck={false}
                onChange={(event) => setId(event.target.value)}
              />
              <small>Letters, numbers, hyphens, and underscores.</small>
            </Field>
            <Field label="MCP server URL">
              <TextInput
                aria-label="MCP server URL"
                type="url"
                value={url}
                required
                maxLength={2048}
                aria-describedby={`${helpId}-endpoint`}
                onChange={(event) => setUrl(event.target.value)}
              />
              <small id={`${helpId}-endpoint`}>
                Use the server's Streamable HTTP endpoint. Localhost refers to
                the Control Plane server.
              </small>
            </Field>
          </div>
        </ProviderSetupSection>
        <ProviderSetupSection
          title="Tools"
          description="Choose which tools can be used from this server."
        >
          <Field label="Allowed tools">
            <TextInput
              aria-label="Allowed tools"
              value={tools}
              required
              placeholder="search, get_*, *_search"
              aria-describedby={`${helpId}-tools`}
              onChange={(event) => setTools(event.target.value)}
            />
            <small id={`${helpId}-tools`}>
              Exact names or * patterns, separated by commas. Use * alone for
              all tools.
            </small>
          </Field>
          <details className="mcp-pattern-help">
            <summary>Pattern matching</summary>
            <p>
              Matching is case-sensitive. A * matches zero or more characters,
              including newly advertised matching tools. Regex and consecutive
              stars are unsupported.
            </p>
          </details>
        </ProviderSetupSection>
        <ProviderSetupSection
          title="Authentication"
          description="Use a personal sign-in or an explicitly assigned shared credential."
        >
          <div className="chat-settings-fields">
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
        </ProviderSetupSection>
      </fieldset>
      <div className="mcp-editor-actions">
        {!oauth &&
        auth !== "none" &&
        (auth === "shared" ? onCredentials : onPersonalCredentials) ? (
          <Button
            disabled={busy}
            onClick={(event) => {
              if (event.currentTarget.form?.reportValidity()) submit(true);
            }}
          >
            Save and choose credential
          </Button>
        ) : null}
        <Button type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" disabled={busy}>
          {editor.id ? "Save changes" : "Add MCP connection"}
        </Button>
      </div>
    </form>
  );
}
