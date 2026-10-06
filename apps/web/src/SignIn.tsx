import { useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import { IconArrowRight, IconShieldLock } from "@tabler/icons-react";
import colossusMark from "@colossus/ui/assets/colossus-mark.svg";
import { ApiFailure, request } from "./api";
import { rememberSignInReturn } from "./navigation";
import { errorMessage } from "./resources";
import type { AuthConfig, Classification } from "./control-api";
export function SignIn({
  config,
  classification,
  error,
  onSignedIn,
}: {
  config: AuthConfig | null;
  classification: Classification | undefined;
  error: string;
  onSignedIn: () => void;
}) {
  const [username, setUsername] = useState(""),
    [password, setPassword] = useState(""),
    [failure, setFailure] = useState(""),
    [busy, setBusy] = useState(false);
  const banner = classification?.enabled ? (
    <div
      className={`control-classification classification-${classification.tone}`}
    >
      {classification.text}
    </div>
  ) : null;
  return (
    <div className="login-layout">
      {banner}
      <main className="sign-in">
        <div className="sign-in-panel">
          <div className="settings-brand">
            <img src={colossusMark} alt="" />
            <strong>Colossus</strong>
          </div>
          <span className="eyebrow">Control Plane</span>
          <h1>Sign in to your Control Plane</h1>
          <p>
            Connect your hosts, operate agent conversations, and follow shared
            work.
          </p>
          {failure || error ? (
            <div className="alert" role="alert">
              {failure || error}
            </div>
          ) : null}
          {config === null ? (
            <p role="status" className="muted">
              Loading sign-in options…
            </p>
          ) : (
            <>
              {config.oidc ? (
                <a
                  className="ui-button ui-button--primary"
                  href={config.oidc.login_url}
                  onClick={rememberSignInReturn}
                >
                  Sign in with {config.oidc.label}
                  <IconArrowRight size={16} aria-hidden="true" />
                </a>
              ) : null}
              {config.local_enabled ? (
                <>
                  <form
                    className="local-sign-in management-form"
                    onSubmit={async (event) => {
                      event.preventDefault();
                      setBusy(true);
                      setFailure("");
                      try {
                        await request("/auth/local", { username, password });
                        setPassword("");
                        onSignedIn();
                      } catch (e) {
                        setFailure(
                          e instanceof ApiFailure &&
                            (e.status === 401 || e.status === 403)
                            ? "Username or password is incorrect, or this account is unavailable."
                            : errorMessage(e),
                        );
                      } finally {
                        setBusy(false);
                        setPassword("");
                      }
                    }}
                  >
                    {config.oidc ? (
                      <span className="login-divider">
                        or use your local account
                      </span>
                    ) : null}
                    <label>
                      <span>Username</span>
                      <TextInput
                        required
                        value={username}
                        onChange={(event) => setUsername(event.target.value)}
                        autoComplete="username"
                      />
                    </label>
                    <label>
                      <span>Password</span>
                      <TextInput
                        required
                        type="password"
                        value={password}
                        onChange={(event) => setPassword(event.target.value)}
                        autoComplete="current-password"
                      />
                    </label>
                    <Button variant="primary" type="submit" disabled={busy}>
                      {busy ? "Signing in…" : "Sign in"}
                    </Button>
                  </form>
                </>
              ) : null}
              {!config.oidc && !config.local_enabled ? (
                <div className="alert">
                  No login provider is enabled. Contact your deployment
                  operator.
                </div>
              ) : null}
            </>
          )}
          <p className="sign-in-foot">
            <IconShieldLock size={16} aria-hidden="true" />
            Your organization manages project access.
          </p>
        </div>
      </main>
      {classification?.enabled && classification.position === "top_and_bottom"
        ? banner
        : null}
    </div>
  );
}
