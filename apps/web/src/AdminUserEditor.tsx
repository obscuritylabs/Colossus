import { useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import { request } from "./api";
import type { AuthConfig, User } from "./control-api";
import { errorMessage } from "./resources";

export function UserEditor({
  user,
  authConfig,
  onCancel,
  onSaved,
}: {
  user?: User | undefined;
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
    [reset, setReset] = useState(false);
  const localIdentity = user?.identities?.some((item) => item.kind === "local");
  async function save() {
    setBusy(true);
    setError("");
    try {
      if (reset && user)
        await request(
          `/api/admin/users/${encodeURIComponent(user.id)}/password`,
          { revision: user.revision, new_password: password },
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
                  ? { username: username.trim(), password }
                  : {}),
                ...(subject.trim() ? { oidc_subject: subject.trim() } : {}),
              },
          undefined,
          user && !reset ? "PATCH" : undefined,
        );
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      setPassword("");
    }
  }
  return (
    <form
      className="management-form control-card"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <header>
        <h3>
          {reset
            ? "Reset local password"
            : user
              ? `Edit ${user.display_name}`
              : "Create user"}
        </h3>
        <Button onClick={onCancel}>Close</Button>
      </header>
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
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
              Active user<small>Changing this retires existing sessions.</small>
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
      <div className="form-actions">
        <Button
          variant="primary"
          type="submit"
          disabled={
            busy ||
            (!reset && !name.trim()) ||
            (!user && !username.trim() && !subject.trim())
          }
        >
          {busy ? "Saving…" : reset ? "Reset password" : "Save user"}
        </Button>
        {user && localIdentity && authConfig?.local_enabled && !reset ? (
          <Button onClick={() => setReset(true)}>Reset local password</Button>
        ) : null}
        <Button onClick={onCancel}>Cancel</Button>
      </div>
    </form>
  );
}
