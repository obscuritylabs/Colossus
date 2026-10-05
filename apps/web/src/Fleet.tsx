import { useEffect, useRef, useState } from "react";
import {
  IconServer,
  IconPlus,
  IconShieldX,
  IconCopy,
  IconCheck,
  IconX,
  IconArrowUpRight,
} from "@tabler/icons-react";
import { request, projectPath, type FleetNode, type Permission } from "./api";
export function Fleet({
  nodes,
  project,
  permissions,
  onRefresh,
  onError,
  hasMore,
  onMore,
}: {
  nodes: FleetNode[];
  project: string;
  permissions: Permission[];
  onRefresh: () => void;
  onError: (error: string) => void;
  hasMore: boolean;
  onMore: () => void;
}) {
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
  return (
    <>
      <div className="section-heading">
        <div>
          <div className="eyebrow">EXECUTION INFRASTRUCTURE</div>
          <h1>Runtime fleet</h1>
          <p>
            Bring your machines. Keep execution under their local authority.
          </p>
        </div>
        <button
          disabled={!permissions.includes("administer")}
          onClick={beginEnrollment}
        >
          <IconPlus size={17} />
          Enroll runtime
        </button>
      </div>
      {nodes.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">
            <IconServer size={31} />
          </div>
          <h2>Your first runtime starts here</h2>
          <p>
            Enroll a CLI daemon or a Desktop runtime to give this project a
            place to execute tasks.
          </p>
          <button
            disabled={!permissions.includes("administer")}
            onClick={beginEnrollment}
          >
            <IconPlus size={17} />
            Enroll a runtime
          </button>
        </div>
      ) : (
        <div className="fleet-grid">
          {nodes.map((fleet) => {
            const status = fleet.node.revoked
              ? "revoked"
              : fleet.presence?.ready
                ? "connected"
                : fleet.presence
                  ? "starting"
                  : "offline";
            return (
              <article className="node-card" key={fleet.node.node_id}>
                <div className="node-header">
                  <div className="node-icon">
                    <IconServer size={23} />
                  </div>
                  <span className={`status status-${status}`}>
                    <span
                      className={`dot ${status === "connected" ? "live" : ""}`}
                    />
                    {status[0]?.toUpperCase() + status.slice(1)}
                  </span>
                </div>
                <h2>{fleet.node.label}</h2>
                <p className="muted mono node-id" title={fleet.node.node_id}>
                  {fleet.node.node_id}
                </p>
                <div className="node-divider" />
                <dl>
                  <dt>Local instance</dt>
                  <dd className="mono truncate" title={fleet.node.instance_id}>
                    {fleet.node.instance_id}
                  </dd>
                  <dt>Allowed roles</dt>
                  <dd>
                    {fleet.node.roles.map((role) => (
                      <span className="tag" key={role}>
                        {role}
                      </span>
                    ))}
                  </dd>
                  <dt>Connection</dt>
                  <dd>Outbound · mutual TLS</dd>
                </dl>
                <details className="node-capabilities">
                  <summary>
                    {fleet.presence?.capabilities.length ?? 0} runtime
                    capabilities
                  </summary>
                  <ul>
                    {fleet.presence?.capabilities.map((capability) => (
                      <li key={capability} className="mono">
                        {capability}
                      </li>
                    ))}
                  </ul>
                </details>
                <footer>
                  <span className="muted">
                    {status === "connected"
                      ? "Ready for tasks"
                      : status === "revoked"
                        ? "Enrollment revoked"
                        : "Tasks wait for reconnect"}
                  </span>
                  <button
                    className="icon-button danger"
                    title="Revoke runtime enrollment"
                    aria-label={`Revoke ${fleet.node.label}`}
                    disabled={
                      !permissions.includes("administer") || fleet.node.revoked
                    }
                    onClick={() => void revoke(fleet)}
                  >
                    <IconShieldX size={18} />
                  </button>
                </footer>
              </article>
            );
          })}
        </div>
      )}
      {hasMore && (
        <button className="secondary" onClick={onMore}>
          Load more runtimes
        </button>
      )}
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
            className="icon-button"
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
              <input readOnly value={invitation.enrollment_url} />
            </label>
            <label>
              One-use invitation
              <input readOnly value={invitation.token} className="mono" />
            </label>
            <button
              className="secondary full-width"
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
            </button>
            <div className="enrollment-instructions">
              <h3>From Desktop</h3>
              <p>
                Open Workspace settings → Cloud. Paste the URL and invitation,
                then confirm the native connection dialog.
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
            <button
              className="full-width"
              onClick={() => {
                setEnrolling(false);
                onRefresh();
              }}
            >
              Done <IconArrowUpRight size={17} />
            </button>
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
              <input
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
              <input
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
            <button
              className="full-width"
              disabled={busy || !label.trim() || !roles.trim()}
            >
              {busy ? "Creating invitation…" : "Create enrollment invitation"}
            </button>
          </form>
        )}
      </dialog>
    </>
  );
}
