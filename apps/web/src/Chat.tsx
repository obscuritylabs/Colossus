import { ThreadBranchDraft, type ForkDraftSource } from "./ThreadBranchDraft";
import { lazy, useEffect, useRef, useState } from "react";
import { Button, WorkWelcome } from "@colossus/ui";
import { WorkSurfaceHeader } from "@colossus/ui/conversation";
import { IconLock, IconPlugConnected } from "@tabler/icons-react";
import {
  projectPath,
  request,
  type FleetNode,
  type Permission,
  type Thread,
  type Task,
} from "./api";
import { useResource, errorMessage } from "./resources";
import { useNavigation } from "./navigation";
import { chatThreadHref, globalHref } from "./routes";
import { LoadState } from "./Home";
import { RunComposer, type RunRequest } from "./RunComposer";
import { ChatConnectionCredential } from "./ChatConnectionCredential";
import "./chat-workspace.css";
import "./thread-inspector.css";
const ThreadDetail = lazy(() =>
  import("./ThreadDetail").then((module) => ({ default: module.ThreadDetail })),
);

interface ChatRuntime {
  enabled: boolean;
  node: FleetNode | null;
}
interface ChatConnection {
  id: string;
  label: string;
  transport: "streamable_http";
  authentication: "personal" | "shared" | "none";
  authenticated: boolean;
  oauth: boolean;
  allowed_tools: string[];
  endpoint?: string;
  credential_binding?: string | null;
}
interface ChatProvider {
  id: string;
  label: string;
  authentication: "personal" | "shared" | "none";
  authenticated: boolean;
  requires_credential: boolean;
  endpoint: string;
  credential_binding: string | null;
}
const STARTERS = [
  "Help me think through a problem",
  "Research a question using my connected sources",
  "Draft a plan and identify what needs my decision",
];

/** Server ownership and each interaction's caller binding remain authoritative. */
export function personalChatPermissions(
  permissions: Permission[],
  owner: string | null | undefined,
  identity: string,
  personalNode: boolean,
): Permission[] {
  return personalNode &&
    owner === identity &&
    permissions.includes("execute") &&
    permissions.includes("control")
    ? [...new Set<Permission>([...permissions, "approve"])]
    : permissions;
}

/** Personal conversations use server-assigned placement and project configuration. */
export function Chat({
  project,
  identity,
  permissions,
  selected: selectedThread,
  detailLoading,
  onChanged,
  view = "conversation",
  onConnections,
  onProjectSettings,
  onCloseConnections,
  forkDraft,
  onCloseFork,
  onFork,
}: {
  project: string;
  identity: string;
  permissions: Permission[];
  selected: Thread | null;
  detailLoading: boolean;
  onChanged: () => void;
  view?: "conversation" | "connections";
  onConnections?: (() => void) | undefined;
  onProjectSettings?: (() => void) | undefined;
  onCloseConnections?: (() => void) | undefined;
  forkDraft?: ForkDraftSource | null;
  onCloseFork?: () => void;
  onFork?: (thread: Thread, source: Task) => void;
}) {
  const navigation = useNavigation();
  const path = `${projectPath(project)}/chat`;
  const runtime = useResource<ChatRuntime>(path, 3000, identity);
  const connections = useResource<{ connections: ChatConnection[] }>(
    view === "connections" ? `${path}/connections` : null,
    3000,
    identity,
  );
  const providers = useResource<{ providers: ChatProvider[] }>(
    view === "connections" ? `${path}/providers` : null,
    3000,
    identity,
  );
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const [connectedNode, setConnectedNode] = useState<FleetNode | null>(null);
  const [draftSeed, setDraftSeed] = useState<{ key: string; text: string }>();
  const [initialRequest, setInitialRequest] = useState<RunRequest>();
  const [createdThread, setCreatedThread] = useState<Thread | null>(null);
  const selected =
    selectedThread ??
    (navigation.route.kind === "chat" &&
    navigation.route.thread === createdThread?.thread_id
      ? createdThread
      : null);
  const [connectionBusy, setConnectionBusy] = useState("");
  const [authorization, setAuthorization] = useState<{
    id: string;
    url: string;
  } | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const node = runtime.data?.node ?? connectedNode;
  const enabled = runtime.data?.enabled === true;
  const canExecute = enabled && permissions.includes("execute");
  const canConnect = canExecute && permissions.includes("control");
  useEffect(() => {
    if (runtime.data?.node) setConnectedNode(runtime.data.node);
    if (runtime.data?.enabled === false) setConnectedNode(null);
  }, [runtime.data]);
  useEffect(() => {
    if (
      authorization &&
      connections.data?.connections.some(
        (item) => item.id === authorization.id && item.authenticated,
      )
    )
      setAuthorization(null);
  }, [authorization, connections.data]);
  async function connect() {
    if (busy || !canExecute) return;
    setBusy(true);
    setError("");
    try {
      const value = await request<ChatRuntime>(`${path}/connect`, {});
      if (!alive.current) return;
      setConnectedNode(value.node);
      runtime.refresh();
      connections.refresh();
    } catch (e) {
      if (alive.current) setError(errorMessage(e));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function create(runRequest: RunRequest) {
    setBusy(true);
    setError("");
    try {
      const result = await request<{ thread: Thread }>(`${path}/threads`, {
        request: runRequest,
      });
      if (!alive.current) return false;
      setInitialRequest(runRequest);
      setCreatedThread(result.thread);
      onChanged();
      navigation.go(chatThreadHref(project, result.thread.thread_id));
      return true;
    } catch (e) {
      if (alive.current) setError(errorMessage(e));
      return false;
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  async function connectionAction(
    connection: ChatConnection,
    disconnect: boolean,
  ) {
    if (
      connectionBusy ||
      !canConnect ||
      connection.authentication !== "personal"
    )
      return;
    setConnectionBusy(connection.id);
    setAuthorization(null);
    setError("");
    try {
      const value = await request<{ authorization_url?: string }>(
        `${path}/connections/${encodeURIComponent(connection.id)}/${disconnect ? "disconnect" : "auth"}`,
        disconnect && !connection.oauth
          ? { credential_binding: connection.credential_binding ?? null }
          : {},
      );
      if (!alive.current) return;
      if (!disconnect) {
        const url = new URL(value.authorization_url ?? "");
        if (
          url.protocol !== "https:" ||
          url.username ||
          url.password ||
          url.hash
        )
          throw new Error(
            "This connection returned an invalid sign-in address.",
          );
        setAuthorization({ id: connection.id, url: url.href });
      }
      connections.refresh();
    } catch (e) {
      if (alive.current) setError(errorMessage(e));
    } finally {
      if (alive.current) setConnectionBusy("");
    }
  }
  async function disconnectProvider(provider: ChatProvider) {
    if (!canConnect || connectionBusy || provider.authentication !== "personal")
      return;
    setConnectionBusy(`provider:${provider.id}`);
    setError("");
    try {
      await request(
        `${path}/providers/${encodeURIComponent(provider.id)}/disconnect`,
        {},
      );
      if (alive.current) {
        providers.refresh();
        runtime.refresh();
      }
    } catch (e) {
      if (alive.current) setError(errorMessage(e));
    } finally {
      if (alive.current) setConnectionBusy("");
    }
  }
  return (
    <section
      className="cloud-chat chat-workspace"
      aria-label="Personal cloud chat"
    >
      <div className="chat-main">
        {error ? (
          <p className="alert" role="alert">
            {error}
            <Button onClick={() => setError("")}>Dismiss</Button>
          </p>
        ) : null}
        <LoadState {...runtime} retry={runtime.refresh} />
        {!runtime.loading && !enabled ? (
          <p className="sync-notice">
            Cloud chat is not configured for this project. A project
            administrator can set up providers, models and controlled
            connections.
          </p>
        ) : null}
        <section
          hidden={view !== "connections"}
          className="chat-connections-page"
          aria-label="Your connections"
        >
          <WorkSurfaceHeader
            title="Your connections"
            statusLabel="Personal accounts"
            actions={
              <>
                {onProjectSettings && permissions.includes("administer") ? (
                  <Button onClick={onProjectSettings}>Project settings</Button>
                ) : null}
                <Button
                  onClick={() => {
                    if (onCloseConnections) onCloseConnections();
                    else
                      navigation.go(
                        selected
                          ? chatThreadHref(project, selected.thread_id)
                          : globalHref("chat", project),
                      );
                  }}
                >
                  Back to chat
                </Button>
              </>
            }
          />
          <div className="chat-connection-content">
            <p className="field-help">
              Models and available tools come from this project. Personal
              connections use your own account.
            </p>
            <h3>Model providers</h3>
            <LoadState {...providers} retry={providers.refresh} />
            {providers.data?.providers.map((provider) => (
              <section
                className="chat-connection"
                key={`provider:${provider.id}`}
                aria-label={`${provider.label} provider`}
              >
                <div>
                  <strong>{provider.label}</strong>
                  <p className="field-help">
                    {provider.authentication === "shared"
                      ? "Project connection"
                      : provider.authentication === "personal"
                        ? "Your account"
                        : "No credential required"}{" "}
                    ·{" "}
                    {provider.authenticated ||
                    provider.authentication === "none"
                      ? "Available"
                      : "Not connected"}
                  </p>
                </div>
                {provider.authentication === "personal" &&
                provider.requires_credential ? (
                  <>
                    <ChatConnectionCredential
                      key={`${project}:${identity}:provider:${provider.id}:${provider.credential_binding}`}
                      path={`${path}/providers/${encodeURIComponent(provider.id)}/credential`}
                      binding={provider.credential_binding}
                      endpoint={provider.endpoint}
                      disabled={!canConnect || Boolean(connectionBusy)}
                      onSaved={() => {
                        providers.refresh();
                        runtime.refresh();
                      }}
                      onError={(e) => setError(errorMessage(e))}
                    />
                    {provider.authenticated ? (
                      <Button
                        disabled={!canConnect || Boolean(connectionBusy)}
                        onClick={() => void disconnectProvider(provider)}
                      >
                        Disconnect provider
                      </Button>
                    ) : null}
                  </>
                ) : null}
              </section>
            ))}
            <h3>MCP connections</h3>
            <LoadState {...connections} retry={connections.refresh} />
            {connections.data?.connections.map((connection) => (
              <section
                className="chat-connection"
                key={connection.id}
                aria-label={connection.label}
              >
                <div>
                  <strong>{connection.label}</strong>
                  <p className="field-help">
                    {connection.authentication === "shared"
                      ? "Project connection"
                      : connection.authentication === "personal"
                        ? "Your account"
                        : "No sign-in required"}{" "}
                    ·{" "}
                    {connection.authenticated ||
                    connection.authentication === "none"
                      ? "Available"
                      : "Not connected"}
                  </p>
                  <small>
                    {connection.allowed_tools.length} allowed tool
                    {connection.allowed_tools.length === 1 ? "" : "s"}
                  </small>
                </div>
                {connection.authentication === "personal" &&
                connection.oauth ? (
                  <Button
                    disabled={Boolean(connectionBusy) || !canConnect}
                    onClick={() =>
                      void connectionAction(
                        connection,
                        connection.authenticated,
                      )
                    }
                  >
                    {connectionBusy === connection.id
                      ? "Working…"
                      : connection.authenticated
                        ? "Disconnect"
                        : "Connect"}
                  </Button>
                ) : null}
                {connection.authentication === "personal" &&
                !connection.oauth ? (
                  <ChatConnectionCredential
                    key={`${project}:${identity}:${connection.id}:${connection.credential_binding}`}
                    path={`${path}/connections/${encodeURIComponent(connection.id)}/credential`}
                    disabled={!canConnect || Boolean(connectionBusy)}
                    binding={connection.credential_binding ?? null}
                    endpoint={connection.endpoint ?? ""}
                    onSaved={() => {
                      runtime.refresh();
                      connections.refresh();
                    }}
                    onError={(e) => setError(errorMessage(e))}
                  />
                ) : null}
                {connection.authentication === "personal" &&
                !connection.oauth &&
                connection.authenticated ? (
                  <Button
                    disabled={
                      !canConnect ||
                      Boolean(connectionBusy) ||
                      !connection.credential_binding
                    }
                    onClick={() => void connectionAction(connection, true)}
                  >
                    Disconnect
                  </Button>
                ) : null}
                {authorization?.id === connection.id ? (
                  <p className="chat-authorization" role="status">
                    <a
                      href={authorization.url}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      Continue sign-in
                    </a>
                    <span>Complete sign-in, then return to your chat.</span>
                  </p>
                ) : null}
              </section>
            ))}
            {!connections.loading &&
            !connections.error &&
            !connections.data?.connections.length ? (
              <p className="field-help">
                No external connections are configured.
              </p>
            ) : null}
          </div>
        </section>
        <div
          className="chat-conversation-workspace"
          hidden={view === "connections"}
        >
          {forkDraft ? (
            <ThreadBranchDraft
              key={forkDraft.key}
              draft={forkDraft}
              kind="fork"
              project={project}
              nodes={node ? [node] : []}
              disabled={
                !canExecute ||
                !node ||
                node.node.revoked ||
                node.node.node_id !== forkDraft.parent.node_id
              }
              onCancel={() => onCloseFork?.()}
              onChanged={onChanged}
              onCreated={(child, submitted) => {
                setInitialRequest(submitted);
                setCreatedThread(child);
                onCloseFork?.();
                navigation.go(chatThreadHref(project, child.thread_id));
              }}
            />
          ) : null}
          <div
            className="chat-conversation-preserved"
            hidden={Boolean(forkDraft)}
          >
            {selected ? (
              <ThreadDetail
                key={selected.thread_id}
                initial={selected}
                project={project}
                nodes={node ? [node] : []}
                permissions={personalChatPermissions(
                  permissions,
                  selected.owner_user_id,
                  identity,
                  node?.node.node_id === selected.node_id,
                )}
                onBack={() => navigation.go(globalHref("chat", project))}
                backHref={globalHref("chat", project)}
                backLabel="All chats"
                personalChat={
                  node
                    ? { owner: identity, node: node.node.node_id }
                    : undefined
                }
                onConnections={onConnections}
                initialComposerRequest={initialRequest}
                onFork={onFork}
                onChanged={onChanged}
              />
            ) : detailLoading ? (
              <p role="status">Opening your chat…</p>
            ) : (
              <div className="chat-new-workspace">
                <WorkSurfaceHeader
                  variant="compact"
                  title="New chat"
                  statusLabel="New work"
                  details={
                    <span className="chat-private-cue">
                      <IconLock size={14} aria-hidden="true" /> Personal chat
                    </span>
                  }
                  actions={
                    <div className="thread-header-actions">
                      {onConnections ? (
                        <Button
                          variant="tertiary"
                          onClick={onConnections}
                          aria-label="Manage your connections"
                        >
                          <IconPlugConnected size={16} aria-hidden="true" />
                        </Button>
                      ) : null}
                    </div>
                  }
                />
                <div
                  className="chat-welcome"
                  tabIndex={0}
                  aria-label="New conversation"
                >
                  <WorkWelcome
                    eyebrow="Your personal conversation"
                    title="What would you like to work on?"
                    description="Think, write, research and work with your connected tools."
                    suggestions={STARTERS}
                    disabled={busy || !node || !canExecute}
                    onSuggestion={(text) =>
                      setDraftSeed({ key: crypto.randomUUID(), text })
                    }
                  />
                </div>
                {node ? (
                  <RunComposer
                    key={node.node.node_id}
                    nodes={[node]}
                    nodeId={node.node.node_id}
                    lockedRuntime
                    runtimeLabel="Personal chat"
                    hostedProject={project}
                    busy={busy}
                    disabled={!canExecute || node.node.revoked}
                    label="Message Colossus"
                    action="Send message"
                    draftSeed={draftSeed}
                    onSubmit={create}
                  />
                ) : enabled ? (
                  <div className="chat-start">
                    <Button
                      variant="primary"
                      disabled={busy || !canExecute}
                      onClick={() => void connect()}
                    >
                      {busy ? "Starting…" : "Start chat"}
                    </Button>
                    {!permissions.includes("execute") ? (
                      <p className="field-help">
                        Chat requires execution access to this project.
                      </p>
                    ) : null}
                  </div>
                ) : null}
              </div>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
