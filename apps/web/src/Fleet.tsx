import { Badge } from "@colossus/ui/components/ui/badge";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  IconServer,
  IconShieldX,
  IconCopy,
  IconCheck,
  IconX,
  IconArrowUpRight,
  IconArrowLeft,
  IconMessageCircle,
  IconDeviceDesktop,
} from "@tabler/icons-react";
import { Button, TextInput } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import {
  request,
  projectPath,
  type FleetNode,
  type Permission,
  type Host,
} from "./api";
import { RouteLink } from "./navigation";
import { agentHref } from "./routes";
const platforms: Record<string, string> = {
  macos: "macOS",
  linux: "Linux",
  windows: "Windows",
};
const platformLabel = (platform: string) => platforms[platform] ?? platform;

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
  onOpenAgent: (nodeId: string) => void;
}) {
  const [hostId, setHostId] = useState<string | null>(null);
  const hostBack = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (hostId !== null) hostBack.current?.focus();
  }, [hostId]);
  const [enrolling, setEnrolling] = useState(false),
    [label, setLabel] = useState(""),
    [roles, setRoles] = useState("primary"),
    [busy, setBusy] = useState(false),
    [copied, setCopied] = useState(false),
    [invitation, setInvitation] = useState<{
      token: string;
      node_id: string;
      enrollment_url: string;
      expires_in: number;
    } | null>(null);
  const input = useRef<HTMLInputElement>(null),
    dialog = useRef<HTMLDialogElement>(null),
    alive = useRef(true),
    enrollmentAttempt = useRef(0);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    if (enrolling) {
      dialog.current?.showModal();
      input.current?.focus();
    } else dialog.current?.close();
  }, [enrolling]);
  function beginEnrollment() {
    enrollmentAttempt.current++;
    setInvitation(null);
    setBusy(false);
    setCopied(false);
    setLabel("");
    setRoles("primary");
    setEnrolling(true);
  }
  function closeEnrollment() {
    enrollmentAttempt.current++;
    setEnrolling(false);
    setInvitation(null);
    setBusy(false);
  }
  async function invite() {
    if (busy) return;
    const attempt = ++enrollmentAttempt.current;
    setBusy(true);
    try {
      const response = await request<{
        token: string;
        node_id: string;
        enrollment_url: string;
        expires_in: number;
      }>(`${projectPath(project)}/invitations`, {
        label,
        roles: roles
          .split(",")
          .map((role) => role.trim())
          .filter(Boolean),
      });
      if (alive.current && attempt === enrollmentAttempt.current) {
        setInvitation(response);
        setCopied(false);
      }
    } catch (error) {
      if (alive.current && attempt === enrollmentAttempt.current)
        onError(
          error instanceof Error
            ? error.message
            : "Enrollment could not be created.",
        );
    } finally {
      if (alive.current && attempt === enrollmentAttempt.current)
        setBusy(false);
    }
  }
  async function revoke(fleet: FleetNode) {
    if (
      !confirm(
        `Revoke ${fleet.node.label}? Its cloud connection will close. Accepted runs continue locally and remain assigned to this runtime.`,
      )
    )
      return;
    try {
      await request(
        `${projectPath(project)}/nodes/${fleet.node.node_id}/revoke`,
        { revision: fleet.node.revision },
      );
      if (alive.current) onRefresh();
    } catch (error) {
      if (alive.current)
        onError(error instanceof Error ? error.message : "Revocation failed.");
    }
  }
  const visibleNodes = nodes.filter(
    (item) =>
      hostId === null ||
      (hostId === "" ? !item.node.host_id : item.node.host_id === hostId),
  );
  const columns = useMemo<DataTableColumn<FleetNode>[]>(
    () => [
      {
        id: "agent",
        label: "Agent / workspace",
        hideable: false,
        rowHeader: true,
        value: (item) =>
          `${item.node.label} ${item.node.workspace_label ?? ""} ${item.node.node_id}`,
        cell: (item) => (
          <div className="catalog-identity">
            <span className="resource-icon">
              <IconServer size={16} aria-hidden="true" />
            </span>
            <div>
              <RouteLink
                href={agentHref(project, item.node.node_id)}
                className="catalog-name"
                onNavigate={() => onOpenAgent(item.node.node_id)}
              >
                {item.node.label}
              </RouteLink>
              <small>
                {item.node.workspace_label ||
                  item.node.workspace_id ||
                  "Workspace inventory not reported"}
              </small>
              <details className="agent-details">
                <summary>Connection details</summary>
                <dl>
                  <dt>Local instance</dt>
                  <dd className="mono">{item.node.instance_id}</dd>
                  <dt>Node</dt>
                  <dd className="mono">{item.node.node_id}</dd>
                  <dt>Transport</dt>
                  <dd>Outbound · mutual TLS</dd>
                  <dt>Capabilities</dt>
                  <dd>
                    {item.presence?.capabilities.join(", ") || "Not reported"}
                  </dd>
                </dl>
              </details>
            </div>
          </div>
        ),
      },
      {
        id: "host",
        label: "Host",
        value: (item) =>
          hosts.find((host) => host.host_id === item.node.host_id)?.label ??
          "Inventory pending",
      },
      {
        id: "status",
        label: "Connection",
        value: (item) =>
          item.node.revoked
            ? "Revoked"
            : item.presence?.ready
              ? "Connected"
              : item.presence
                ? "Starting"
                : "Offline",
        cell: (item) => {
          const status = item.node.revoked
            ? "revoked"
            : item.presence?.ready
              ? "connected"
              : item.presence
                ? "starting"
                : "offline";
          return (
            <Badge className={`status status-${status}`}>
              <span className={`dot ${status === "connected" ? "live" : ""}`} />
              {status[0]!.toUpperCase() + status.slice(1)}
            </Badge>
          );
        },
        filter: {
          label: "Filter agents by connection",
          options: [
            { value: "", label: "All connections" },
            { value: "online", label: "Online" },
            { value: "offline", label: "Offline" },
            { value: "revoked", label: "Revoked" },
          ],
          matches: (item, value) =>
            !value ||
            (value === "revoked"
              ? item.node.revoked
              : !item.node.revoked &&
                (value === "online"
                  ? Boolean(item.presence?.ready)
                  : !item.presence?.ready)),
        },
      },
      {
        id: "roles",
        label: "Allowed roles",
        value: (item) => item.node.roles.join(", "),
      },
      {
        id: "actions",
        label: "Actions",
        value: () => "",
        sortable: false,
        hideable: false,
        cell: (item) => (
          <div className="agent-row-actions">
            <RouteLink
              className="ui-button ui-button--tertiary"
              href={agentHref(project, item.node.node_id, "threads")}
              aria-label={`Open threads on ${item.node.label}`}
              onNavigate={() => onOpenAgent(item.node.node_id)}
            >
              <IconMessageCircle size={16} aria-hidden="true" />
            </RouteLink>
            {permissions.includes("administer") ? (
              <Button
                variant="tertiary"
                aria-label={`Revoke ${item.node.label}`}
                disabled={item.node.revoked}
                onClick={() => void revoke(item)}
              >
                <IconShieldX size={16} aria-hidden="true" />
              </Button>
            ) : null}
          </div>
        ),
      },
    ],
    [hosts, onOpenAgent, permissions, project],
  );
  return (
    <>
      <section
        className="managed-settings-body catalog-settings host-settings"
        aria-labelledby="fleet-heading"
      >
        <header className="catalog-heading">
          <div>
            <h2 id="fleet-heading">Fleet</h2>
            <p>
              Hosts group your connected Desktop workspaces and independent CLI
              agents.
            </p>
          </div>
          {permissions.includes("administer") ? (
            <Button variant="primary" onClick={beginEnrollment}>
              Enroll agent
            </Button>
          ) : null}
        </header>
        <p className="catalog-summary">
          {hosts.length} loaded hosts · {nodes.length} loaded agents ·{" "}
          {
            nodes.filter((item) => item.presence?.ready && !item.node.revoked)
              .length
          }{" "}
          online
        </p>
        <details className="host-directory" open={hostId !== null}>
          <summary>
            Browse hosts{hostId !== null ? " · Host filter active" : ""}
          </summary>
          {hostId !== null ? (
            <Button
              ref={hostBack}
              variant="tertiary"
              className="back"
              onClick={() => {
                const previous = hostId;
                setHostId(null);
                requestAnimationFrame(() =>
                  document
                    .querySelector<HTMLButtonElement>(
                      `[data-host-id="${CSS.escape(previous)}"]`,
                    )
                    ?.focus(),
                );
              }}
            >
              <IconArrowLeft size={16} aria-hidden="true" />
              All hosts
            </Button>
          ) : null}
          {hostId === null ? (
            <div className="host-grid">
              {hosts.map((host) => {
                const agents = nodes.filter(
                    (item) =>
                      item.node.host_id === host.host_id && !item.node.revoked,
                  ),
                  online = agents.filter((item) => item.presence?.ready).length;
                return (
                  <button
                    type="button"
                    className="host-card"
                    data-host-id={host.host_id}
                    key={host.host_id}
                    onClick={() => setHostId(host.host_id)}
                  >
                    <span className="host-card-icon">
                      <IconDeviceDesktop size={22} aria-hidden="true" />
                    </span>
                    <span className="host-card-body">
                      <strong>{host.label}</strong>
                      <small>
                        {platformLabel(host.platform) ||
                          "Platform not reported"}{" "}
                        ·{" "}
                        {host.deployment_kind === "cli"
                          ? "CLI"
                          : host.deployment_kind === "desktop"
                            ? "Desktop"
                            : host.deployment_kind || "Runtime host"}
                      </small>
                      <span>
                        {agents.length} loaded agent
                        {agents.length === 1 ? "" : "s"} · {online} online
                      </span>
                      {agents.length ? (
                        <small>
                          {agents
                            .slice(0, 3)
                            .map((item) => item.node.label)
                            .join(" · ")}
                          {agents.length > 3 ? "…" : ""}
                        </small>
                      ) : null}
                      <small>
                        {host.last_seen_at
                          ? `Last seen ${new Date(host.last_seen_at * 1000).toLocaleString()}`
                          : "Waiting for host inventory"}
                      </small>
                    </span>
                    <IconArrowUpRight size={16} aria-hidden="true" />
                  </button>
                );
              })}
              {nodes.some((item) => !item.node.host_id) ? (
                <button
                  type="button"
                  className="host-card"
                  data-host-id=""
                  onClick={() => setHostId("")}
                >
                  <span className="host-card-icon">
                    <IconServer size={22} aria-hidden="true" />
                  </span>
                  <span className="host-card-body">
                    <strong>Agents without host inventory</strong>
                    <small>
                      Existing enrollments that have not reported a host
                    </small>
                    <span>
                      {nodes.filter((item) => !item.node.host_id).length} agents
                    </span>
                  </span>
                  <IconArrowUpRight size={16} aria-hidden="true" />
                </button>
              ) : null}
              {!hosts.length && !nodes.length ? (
                <div className="empty-state">
                  <IconDeviceDesktop size={28} aria-hidden="true" />
                  <h3>Connect your first host</h3>
                  <p>
                    Enroll a Desktop workspace or CLI agent. Its host and
                    workspace inventory will appear after it connects.
                  </p>
                </div>
              ) : null}
            </div>
          ) : (
            <p className="fleet-host-context">
              <IconDeviceDesktop size={18} aria-hidden="true" />
              {hosts.find((host) => host.host_id === hostId)?.label ??
                "Agents without host inventory"}
            </p>
          )}
          {hasMoreHosts && hostId === null ? (
            <Button className="load-more" onClick={onMoreHosts}>
              Load more hosts
            </Button>
          ) : null}
        </details>
      </section>
      <section
        className="managed-settings-body catalog-settings runtime-settings"
        aria-labelledby="agents-heading"
      >
        <header className="catalog-heading">
          <div>
            <h2 id="agents-heading">Agents and workspaces</h2>
            <p>
              Open an agent to view its conversations. Each workspace keeps its
              own execution authority.
            </p>
          </div>
        </header>
        <DataTable
          key={hostId ?? "fleet"}
          data={visibleNodes}
          columns={columns}
          getRowId={(item) => item.node.node_id}
          label="Fleet agents"
          itemLabel="agents"
          search={{ columnId: "agent", label: "Search agents" }}
          empty={
            <div className="empty-state">
              <h3>No matching agents</h3>
              <p>
                Connect an enrolled agent on this host or choose another host.
              </p>
            </div>
          }
          footer={
            hasMore ? (
              <div className="task-history-footer">
                <p>
                  Filters apply to loaded agents. Load more to include the rest
                  of your fleet.
                </p>
                <Button onClick={onMore}>Load more agents</Button>
              </div>
            ) : undefined
          }
        />
      </section>
      <dialog
        aria-label="Runtime enrollment"
        ref={dialog}
        onCancel={closeEnrollment}
        onClose={() => {
          if (!dialog.current?.open) closeEnrollment();
        }}
        className="enrollment-dialog"
      >
        <div className="dialog-heading">
          <div className="node-icon">
            <IconServer size={23} />
          </div>
          <button
            className="ui-icon-button"
            aria-label="Close enrollment"
            onClick={closeEnrollment}
          >
            <IconX size={20} />
          </button>
        </div>
        <h2>{invitation ? "Connect your runtime" : "Enroll a runtime"}</h2>
        <p className="muted">
          {invitation
            ? "This invitation expires in ten minutes and can enroll one runtime."
            : "Choose a name and an additional role ceiling for this project's execution target."}
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
              {copied ? <IconCheck size={17} /> : <IconCopy size={17} />}{" "}
              {copied ? "Invitation copied" : "Copy invitation"}
            </Button>
            <div className="enrollment-instructions">
              <h3>From Desktop</h3>
              <p>
                Open Workspace settings → Control Plane. Paste the URL and
                invitation, then confirm the native connection dialog.
              </p>
              <h3>From the CLI</h3>
              <code>
                colossus cloud enroll --enrollment-url URL --local-config
                LOCAL.json
              </code>
              <p>
                Paste the invitation when prompted. Start the persistent
                connector with{" "}
                <code>colossus cloud run --local-config LOCAL.json</code>.
              </p>
            </div>
            <Button
              variant="primary"
              className="full-width"
              onClick={() => {
                setEnrolling(false);
                onRefresh();
              }}
            >
              Done <IconArrowUpRight size={17} />
            </Button>
          </>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void invite();
            }}
          >
            <label htmlFor="node-label">
              Runtime name
              <TextInput
                ref={input}
                id="node-label"
                required
                maxLength={128}
                autoComplete="off"
                placeholder="e.g. Development workstation"
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </label>
            <label htmlFor="node-roles">
              Allowed roles
              <TextInput
                id="node-roles"
                required
                value={roles}
                onChange={(event) => setRoles(event.target.value)}
                pattern={"[a-zA-Z0-9_, \\-]+"}
                maxLength={512}
              />
              <small>
                The local application grant further limits these roles and their
                tools.
              </small>
            </label>
            <Button
              type="submit"
              variant="primary"
              className="full-width"
              disabled={busy || !label.trim() || !roles.trim()}
            >
              {busy ? "Creating invitation…" : "Create enrollment invitation"}
            </Button>
          </form>
        )}
      </dialog>
    </>
  );
}
