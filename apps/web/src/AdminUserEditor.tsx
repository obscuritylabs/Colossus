import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import { ApiFailure, request } from "./api";
import type { AuthConfig, User } from "./control-api";
import { errorMessage } from "./resources";
import { useSettingsNavigationGuard } from "./navigation";

export function UserEditor({
  user,
  authConfig,
  onCancel,
  onSaved,
  passwordReset = false,
  showHeading = true,
  onBusyChange,
  onDirtyChange,
  onReload,
}: {
  user?: User | undefined;
  passwordReset?: boolean;
  showHeading?: boolean;
  onBusyChange?: ((busy: boolean) => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  onReload?: (() => void) | undefined;
  authConfig: AuthConfig | null;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(user?.display_name ?? ""),
    [email, setEmail] = useState(user?.email ?? ""),
    [username, setUsername] = useState(""),
    [password, setPassword] = useState(""),
    [subject, setSubject] = useState(""),
    [active, setActive] = useState(user?.active ?? true),
    [admin, setAdmin] = useState(user?.is_admin ?? false),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [reset, setReset] = useState(passwordReset);
  const localDirty = useSettingsNavigationGuard();
  const dirty = onDirtyChange ?? localDirty;
  const changed = useRef(false);
  const [needsReload, setNeedsReload] = useState(false);
  useEffect(() => {
    onBusyChange?.(busy);
    dirty(changed.current || busy || needsReload);
    return () => {
      onBusyChange?.(false);
      dirty(false);
    };
  }, [busy, needsReload, dirty, onBusyChange]);
  const pending = useRef(false),
    alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const localIdentity = user?.identities?.some((item) => item.kind === "local");
  async function save() {
    if (pending.current || needsReload) return;
    if (
      !reset &&
      (!name.trim() || new TextEncoder().encode(name.trim()).length > 256)
    ) {
      setError("Enter a display name of at most 256 UTF-8 bytes.");
      return;
    }
    if (
      ((!user || reset) && username.trim() && !authConfig?.local_enabled) ||
      (reset && (!user || !localIdentity || !authConfig?.local_enabled))
    ) {
      setError("Local password management is unavailable for this account.");
      return;
    }
    if (
      !user &&
      !(authConfig?.local_enabled && username.trim()) &&
      !(authConfig?.oidc && subject.trim())
    ) {
      setError("Provide at least one enabled login identity.");
      return;
    }
    if ((reset || (!user && username.trim())) && password.length < 15) {
      setError("Use at least 15 characters for the password.");
      return;
    }
    const submittedPassword = password;
    setPassword("");
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      if (reset && user)
        await request(
          `/api/admin/users/${encodeURIComponent(user.id)}/password`,
          { revision: user.revision, new_password: submittedPassword },
        );
      else
        await request(
          user
            ? `/api/admin/users/${encodeURIComponent(user.id)}`
            : "/api/admin/users",
          user
            ? {
                revision: user.revision,
                display_name: name.trim(),
                active,
                is_admin: admin,
              }
            : {
                display_name: name.trim(),
                email: email.trim() || null,
                is_admin: admin,
                ...(username.trim()
                  ? { username: username.trim(), password: submittedPassword }
                  : {}),
                ...(subject.trim() ? { oidc_subject: subject.trim() } : {}),
              },
          undefined,
          user && !reset ? "PATCH" : undefined,
        );
      if (alive.current) {
        changed.current = false;
        dirty(false);
        onSaved();
      }
    } catch (e) {
      if (alive.current) {
        const uncertain = !(e instanceof ApiFailure) || e.status >= 500;
        setNeedsReload(
          uncertain || (e instanceof ApiFailure && e.status === 409),
        );
        setError(
          uncertain
            ? "The save could not be confirmed. Return to Users and review the current account before making another change."
            : e instanceof ApiFailure && e.status === 409
              ? "This account changed. Reload its current version before saving again."
              : errorMessage(e),
        );
      }
    } finally {
      pending.current = false;
      if (alive.current) {
        setBusy(false);
        setPassword("");
      }
    }
  }
  return (
    <form
      className="management-form control-card"
      aria-busy={busy}
      aria-label={
        reset ? "Reset local password" : user ? "Edit user" : "Create user"
      }
      onChange={() => {
        changed.current = true;
        dirty(true);
      }}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      {showHeading ? (
        <header>
          <h3>
            {reset
              ? "Reset local password"
              : user
                ? `Edit ${user.display_name}`
                : "Create user"}
          </h3>
          <Button disabled={busy} onClick={onCancel}>
            Close
          </Button>
        </header>
      ) : null}
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      {needsReload && user && onReload ? (
        <Button disabled={busy} onClick={onReload}>
          Reload current user
        </Button>
      ) : null}
      <fieldset
        className="settings-editor-fields"
        disabled={busy || needsReload}
      >
        {reset ? (
          <>
            <p className="field-help">
              Resetting the password invalidates this user’s existing sessions.
            </p>
            <label>
              <span>New password</span>
              <TextInput
                type="password"
                autoComplete="new-password"
                required
                minLength={15}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
          </>
        ) : (
          <>
            <label>
              <span>Display name</span>
              <TextInput
                required
                maxLength={200}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            {!user ? (
              <>
                <label>
                  <span>Email · optional</span>
                  <TextInput
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    autoComplete="off"
                  />
                </label>
                {authConfig?.local_enabled ? (
                  <div className="form-pair">
                    <label>
                      <span>Local username</span>
                      <TextInput
                        value={username}
                        onChange={(e) => setUsername(e.target.value)}
                        autoComplete="off"
                      />
                    </label>
                    <label>
                      <span>Local password</span>
                      <TextInput
                        type="password"
                        minLength={15}
                        required={Boolean(username.trim())}
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        autoComplete="new-password"
                      />
                    </label>
                  </div>
                ) : null}
                {authConfig?.oidc ? (
                  <label>
                    <span>OIDC subject</span>
                    <TextInput
                      value={subject}
                      onChange={(e) => setSubject(e.target.value)}
                      autoComplete="off"
                    />
                    <small>
                      Exact subject from {authConfig.oidc.label}. Emails do not
                      automatically link identities.
                    </small>
                  </label>
                ) : null}
                <p className="field-help">
                  Provide at least one enabled login identity.
                </p>
              </>
            ) : (
              <label className="checkbox-field">
                <input
                  type="checkbox"
                  checked={active}
                  onChange={(e) => setActive(e.target.checked)}
                />
                Active user
                <small>Changing this retires existing sessions.</small>
              </label>
            )}
            <label className="checkbox-field">
              <input
                type="checkbox"
                checked={admin}
                onChange={(e) => setAdmin(e.target.checked)}
              />
              Control Plane administrator
            </label>
            <p className="field-help">
              Administrators can read and manage projects. Running tasks and
              approving actions still require an explicit project role.
            </p>
          </>
        )}
      </fieldset>
      <div className="form-actions">
        <Button
          variant="primary"
          type="submit"
          disabled={
            busy ||
            needsReload ||
            (!reset && !name.trim()) ||
            (!user && !username.trim() && !subject.trim())
          }
        >
          {busy ? "Saving…" : reset ? "Reset password" : "Save user"}
        </Button>
        {user && localIdentity && authConfig?.local_enabled && !reset ? (
          <Button
            disabled={busy || needsReload}
            onClick={() => {
              if (
                changed.current &&
                !window.confirm(
                  "Discard profile changes and reset this user's password?",
                )
              )
                return;
              setName(user.display_name);
              setActive(user.active);
              setAdmin(user.is_admin);
              setPassword("");
              changed.current = false;
              dirty(false);
              setReset(true);
            }}
          >
            Reset local password
          </Button>
        ) : null}
        <Button disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
