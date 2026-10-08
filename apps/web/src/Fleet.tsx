import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import {
  IconArrowUpRight,
  IconCheck,
  IconCopy,
  IconDeviceDesktop,
  IconFolder,
  IconX,
} from "@tabler/icons-react";
import {
  request,
  projectPath,
  type FleetNode,
  type Host,
  type Permission,
} from "./api";
import { RouteLink } from "./navigation";
import { agentHref, hostHref } from "./routes";
import {
  hostConnection,
  platformName,
  workspaceName,
} from "./workspace-navigation";

export function Fleet({
  nodes,
  hosts,
  project,
  permissions,
  onRefresh,
  onError,
  hasMore,
  hasMoreHosts,
  onMoreHosts,
  onMore,
  onOpenAgent,
  onOpenHost,
}: {
  nodes: FleetNode[];
  hosts: Host[];
  project: string;
  permissions: Permission[];
  onRefresh: () => void;
  onError: (error: string) => void;
  hasMore: boolean;
  hasMoreHosts: boolean;
  onMoreHosts: () => void;
  onMore: () => void;
  onOpenAgent: (id: string) => void;
  onOpenHost: (id: string) => void;
}) {
  const [query, setQuery] = useState(""),
    [enrolling, setEnrolling] = useState(false),
    [roles, setRoles] = useState("primary"),
    [busy, setBusy] = useState(false),
    [copied, setCopied] = useState(false);
  const [invitation, setInvitation] = useState<{
    token: string;
    node_id: string;
    enrollment_url: string;
    expires_in: number;
  } | null>(null);
  const dialog = useRef<HTMLDialogElement>(null),
    roleInput = useRef<HTMLInputElement>(null),
    alive = useRef(true),
    attempt = useRef(0);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    if (enrolling) {
      dialog.current?.showModal();
      roleInput.current?.focus();
    } else dialog.current?.close();
  }, [enrolling]);
  function close() {
    attempt.current++;
    setEnrolling(false);
    setInvitation(null);
    setBusy(false);
  }
  function begin() {
    attempt.current++;
    setInvitation(null);
    setRoles("primary");
    setCopied(false);
    setBusy(false);
    setEnrolling(true);
  }
  async function invite() {
    if (busy) return;
    const generation = ++attempt.current;
    setBusy(true);
    try {
      const value = await request<NonNullable<typeof invitation>>(
        `${projectPath(project)}/invitations`,
        {
          label: "Workspace enrollment",
          roles: roles
            .split(",")
            .map((role) => role.trim())
            .filter(Boolean),
        },
      );
      if (alive.current && attempt.current === generation) {
        setInvitation(value);
        setCopied(false);
      }
    } catch (e) {
      if (alive.current && attempt.current === generation)
        onError(
          e instanceof Error ? e.message : "Invitation could not be created.",
        );
    } finally {
      if (alive.current && attempt.current === generation) setBusy(false);
    }
  }
  const search = query.trim().toLowerCase();
  const visible = hosts.filter(
    (host) =>
      host.project_id === project &&
      `${host.label} ${platformName(host.platform)}`
        .toLowerCase()
        .includes(search),
  );
  const unassigned = nodes.filter(
    (item) => item.node.project_id === project && !item.node.host_id,
  );
  return (
    <section
      className="managed-settings-body fleet-view"
      aria-labelledby="fleet-heading"
    >
      <header className="fleet-heading">
        <div>
          <h1 id="fleet-heading">Fleet</h1>
          <p>Open a host to work with its connected workspaces.</p>
        </div>
        {permissions.includes("administer") ? (
          <Button variant="primary" onClick={begin}>
            Connect workspace
          </Button>
        ) : null}
      </header>
      <div className="fleet-toolbar">
        <label>
          <span className="sr-only">Search hosts</span>
          <TextInput
            type="search"
            aria-label="Search hosts"
            placeholder="Search hosts"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <span>
          {hosts.length} host{hosts.length === 1 ? "" : "s"}
        </span>
      </div>
      <ul className="fleet-host-list" aria-label="Fleet hosts">
        {visible.map((host) => {
          const state = hostConnection(host, nodes);
          return (
            <li key={host.host_id}>
              <RouteLink
                className="fleet-host-row"
                href={hostHref(project, host.host_id)}
                onNavigate={() => onOpenHost(host.host_id)}
                aria-label={`Open host ${host.label}`}
              >
                <span className="fleet-host-icon">
                  <IconDeviceDesktop size={24} aria-hidden="true" />
                </span>
                <span className="fleet-host-name">
                  <strong>{host.label}</strong>
                  <small>
                    {platformName(host.platform)} ·{" "}
                    {host.deployment_kind === "desktop" ? "Desktop" : "CLI"}
                  </small>
                </span>
                <span className="fleet-host-workspaces">
                  <strong>
                    {state.total} workspace{state.total === 1 ? "" : "s"}
                  </strong>
                  <small>
                    {state.ready} connected
                    {hasMore ? " · loaded inventory" : ""}
                  </small>
                </span>
                <span className="fleet-host-status">
                  <span className={`dot ${state.ready ? "live" : ""}`} />
                  {state.label}
                </span>
                <IconArrowUpRight size={18} aria-hidden="true" />
              </RouteLink>
            </li>
          );
        })}
      </ul>
      {!visible.length ? (
        <div className="empty-state">
          <IconDeviceDesktop size={28} aria-hidden="true" />
          <h2>
            {search ? "No matching hosts" : "Connect your first workspace"}
          </h2>
          <p>
            {search
              ? "Try another host name or operating system."
              : "Its host will appear here when the workspace connects."}
          </p>
        </div>
      ) : null}
      {hasMoreHosts ? (
        <Button variant="secondary" onClick={onMoreHosts}>
          More hosts
        </Button>
      ) : null}
      {hasMore ? (
        <div className="fleet-inventory-more">
          <span>Workspace counts reflect the loaded inventory.</span>
          <Button variant="tertiary" onClick={onMore}>
            More workspace connections
          </Button>
        </div>
      ) : null}
      {unassigned.length ? (
        <details className="fleet-unassigned">
          <summary>Awaiting host information · {unassigned.length}</summary>
          <ul>
            {unassigned.map((item) => (
              <li key={item.node.node_id}>
                <RouteLink
                  href={agentHref(project, item.node.node_id)}
                  onNavigate={() => onOpenAgent(item.node.node_id)}
                >
                  <IconFolder size={16} aria-hidden="true" />
                  {workspaceName(item)}
                </RouteLink>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
      <dialog
        ref={dialog}
        aria-label="Workspace enrollment"
        className="enrollment-dialog"
        onCancel={close}
        onClose={() => {
          if (!dialog.current?.open) close();
        }}
      >
        <div className="dialog-heading">
          <span className="node-icon">
            <IconFolder size={23} aria-hidden="true" />
          </span>
          <button
            type="button"
            className="ui-icon-button"
            aria-label="Close enrollment"
            onClick={close}
          >
            <IconX size={20} />
          </button>
        </div>
        <h2>
          {invitation ? "Connect a workspace" : "Create a workspace invitation"}
        </h2>
        <p className="muted">
          {invitation
            ? "This one-use invitation expires in ten minutes."
            : "The workspace and host names appear automatically after connection. Review the role ceiling for this project."}
        </p>
        {invitation ? (
          <>
            <label>
              Enrollment URL
              <TextInput readOnly value={invitation.enrollment_url} />
            </label>
            <label>
              One-use invitation
              <TextInput readOnly value={invitation.token} className="mono" />
            </label>
            <Button
              className="full-width"
              onClick={() =>
                void navigator.clipboard
                  .writeText(invitation.token)
                  .then(() => setCopied(true))
                  .catch(() =>
                    onError(
                      "Clipboard unavailable. Select and copy the invitation field.",
                    ),
                  )
              }
            >
              {copied ? <IconCheck size={17} /> : <IconCopy size={17} />}
              {copied ? "Invitation copied" : "Copy invitation"}
            </Button>
            <div className="enrollment-instructions">
              <h3>In Desktop</h3>
              <p>
                Select the workspace, open its Control Plane settings, paste the
                URL and invitation, then review the native confirmation.
              </p>
              <details>
                <summary>CLI connection</summary>
                <code>
                  colossus control-plane enroll --enrollment-url URL
                  --local-config LOCAL.json
                </code>
                <p>
                  Paste the invitation when prompted, then start its connector
                  with{" "}
                  <code>
                    colossus control-plane run --local-config LOCAL.json
                  </code>
                  .
                </p>
              </details>
            </div>
            <Button
              variant="primary"
              className="full-width"
              onClick={() => {
                close();
                onRefresh();
              }}
            >
              Done
            </Button>
          </>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void invite();
            }}
          >
            <label>
              Allowed roles
              <TextInput
                ref={roleInput}
                required
                maxLength={128}
                value={roles}
                onChange={(event) => setRoles(event.target.value)}
                placeholder="primary"
                autoComplete="off"
              />
            </label>
            <p className="muted">
              These roles intersect the workspace's own permissions. An
              invitation does not expand its local authority.
            </p>
            <Button
              type="submit"
              variant="primary"
              className="full-width"
              disabled={busy || !roles.trim()}
            >
              {busy ? "Creating…" : "Create invitation"}
            </Button>
          </form>
        )}
      </dialog>
    </section>
  );
}
