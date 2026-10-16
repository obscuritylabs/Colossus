import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import {
  IconArrowRight,
  IconArrowUpRight,
  IconCheck,
  IconEye,
  IconEyeOff,
  IconLock,
  IconServer,
  IconShieldLock,
} from "@tabler/icons-react";
import mark from "@colossus/ui/assets/colossus-mark.svg";
import {
  messageOf,
  normalizeOrigin,
  type AuthConfiguration,
  type Identity,
  type MobileClient,
} from "./client";

export function Login({
  client,
  onLogin,
  onPreview,
  notice,
}: {
  client: MobileClient;
  onLogin: (
    identity: Identity,
    origin: string,
    auth: AuthConfiguration,
  ) => void;
  onPreview?: () => void;
  notice: string;
}) {
  const [origin, setOrigin] = useState(() => {
    try {
      return localStorage.getItem("colossus.mobile.origin") ?? "";
    } catch {
      return "";
    }
  });
  const [auth, setAuth] = useState<AuthConfiguration | null>(null);
  const [username, setUsername] = useState(""),
    [password, setPassword] = useState("");
  const [visible, setVisible] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const usernameField = useRef<HTMLInputElement>(null),
    passwordField = useRef<HTMLInputElement>(null),
    serverField = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (auth) usernameField.current?.focus();
  }, [auth]);
  async function connect() {
    setBusy(true);
    setError("");
    try {
      const normalized = normalizeOrigin(origin, import.meta.env.DEV);
      const result = await client.connect(normalized);
      if (typeof result.local_enabled !== "boolean")
        throw new Error(
          "This server does not provide a compatible Colossus sign-in service.",
        );
      setOrigin(normalized);
      setAuth(result);
      try {
        localStorage.setItem("colossus.mobile.origin", normalized);
      } catch {
        /* Remembering an address is optional. */
      }
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }
  async function signIn() {
    setBusy(true);
    setError("");
    const entered = password;
    setPassword("");
    setVisible(false);
    try {
      const identity = await client.login(username.trim(), entered);
      if (auth) onLogin(identity, origin, auth);
    } catch (failure) {
      setError(messageOf(failure));
      requestAnimationFrame(() => passwordField.current?.focus());
    } finally {
      setBusy(false);
    }
  }
  function changeServer() {
    setAuth(null);
    setPassword("");
    setUsername("");
    setVisible(false);
    setError("");
    requestAnimationFrame(() => serverField.current?.focus());
  }
  return (
    <main className="mobile-login">
      {client.preview && (
        <p className="mobile-preview-banner" role="status">
          Development preview · Sample data
        </p>
      )}
      {auth?.classification?.enabled && (
        <div className="mobile-classification">{auth.classification.text}</div>
      )}
      <div className="mobile-login-content">
        <header className="mobile-brand">
          <img src={mark} alt="" />
          <strong>Colossus</strong>
        </header>
        <p className="mobile-login-tagline">Your agents. Wherever you are.</p>
        <ol className="mobile-login-steps" aria-label="Sign-in steps">
          <li aria-current={!auth ? "step" : undefined}>
            <span>
              {auth ? <IconCheck size={14} aria-hidden="true" /> : "1"}
            </span>{" "}
            Connect
          </li>
          <li aria-current={auth ? "step" : undefined}>
            <span>2</span> Sign in
          </li>
        </ol>
        <section
          className="mobile-auth-card"
          aria-labelledby="login-title"
          aria-busy={busy}
        >
          <header>
            <h1 id="login-title">
              {auth ? "Sign in to Colossus" : "Connect to Colossus"}
            </h1>
            <p className="mobile-muted">
              {auth
                ? "Welcome back. Continue with your organization’s account."
                : "Enter your organization’s Control Plane address to get started."}
            </p>
          </header>
          {notice && (
            <p role="status" className="mobile-notice">
              {notice}
            </p>
          )}
          {error && (
            <p role="alert" id="login-error" className="mobile-error">
              {error}
            </p>
          )}
          {!auth ? (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void connect();
              }}
            >
              <label htmlFor="server">Server address</label>
              <TextInput
                ref={serverField}
                id="server"
                type="url"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                autoComplete="url"
                inputMode="url"
                placeholder="https://colossus.example.com"
                value={origin}
                disabled={busy}
                onChange={(event) => setOrigin(event.target.value)}
                aria-describedby={
                  error ? "server-help login-error" : "server-help"
                }
                aria-invalid={Boolean(error)}
                required
              />
              <p id="server-help" className="mobile-caption">
                Use the address provided by your administrator.
              </p>
              <Button
                type="submit"
                variant="primary"
                disabled={busy || !origin.trim()}
              >
                {busy ? "Connecting…" : "Continue"}
                <IconArrowRight size={17} aria-hidden="true" />
              </Button>
            </form>
          ) : (
            <>
              <div className="mobile-server-selection">
                {origin.startsWith("https:") ? (
                  <IconLock size={17} aria-hidden="true" />
                ) : (
                  <IconServer size={17} aria-hidden="true" />
                )}
                <div>
                  <strong>{new URL(origin).host}</strong>
                  <span>
                    {origin.startsWith("https:")
                      ? "Secure connection"
                      : "Local development · HTTP"}
                  </span>
                </div>
                <Button
                  variant="tertiary"
                  disabled={busy}
                  onClick={changeServer}
                >
                  Change<span className="shared-sr-only"> server</span>
                </Button>
              </div>
              {auth.local_enabled && (
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    void signIn();
                  }}
                >
                  <label htmlFor="username">Username</label>
                  <TextInput
                    ref={usernameField}
                    id="username"
                    autoComplete="username"
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    value={username}
                    maxLength={256}
                    disabled={busy}
                    onChange={(event) => setUsername(event.target.value)}
                    required
                  />
                  <label htmlFor="password">Password</label>
                  <div className="mobile-password-field">
                    <TextInput
                      ref={passwordField}
                      id="password"
                      type={visible ? "text" : "password"}
                      autoComplete="current-password"
                      value={password}
                      maxLength={1024}
                      disabled={busy}
                      onChange={(event) => setPassword(event.target.value)}
                      aria-describedby={error ? "login-error" : undefined}
                      required
                    />
                    <Button
                      variant="tertiary"
                      aria-label={visible ? "Hide password" : "Show password"}
                      aria-pressed={visible}
                      disabled={busy}
                      onClick={() => setVisible((value) => !value)}
                    >
                      {visible ? (
                        <IconEyeOff size={19} aria-hidden="true" />
                      ) : (
                        <IconEye size={19} aria-hidden="true" />
                      )}
                    </Button>
                  </div>
                  <Button
                    type="submit"
                    variant="primary"
                    disabled={busy || !username.trim() || !password}
                  >
                    {busy ? "Signing in…" : "Sign in"}
                    <IconArrowRight size={17} aria-hidden="true" />
                  </Button>
                </form>
              )}
              {auth.oidc && (
                <section className="mobile-sso">
                  <h2>Organization sign-in</h2>
                  <Button
                    onClick={() => {
                      void client
                        .openBrowser()
                        .catch((failure) => setError(messageOf(failure)));
                    }}
                  >
                    <IconArrowUpRight size={17} aria-hidden="true" />
                    Open Control Plane
                  </Button>
                  <p className="mobile-caption">
                    Continue with {auth.oidc.label} in your browser. Browser
                    sign-in stays in Control Plane; it does not sign in this app
                    yet.
                  </p>
                </section>
              )}
              {!auth.local_enabled && !auth.oidc && (
                <p role="alert">
                  This server has no sign-in method enabled. Contact your
                  administrator.
                </p>
              )}
            </>
          )}
        </section>
        <p className="mobile-caption mobile-login-help">
          Need an account or a password reset? Contact your administrator.
        </p>
        {onPreview && (
          <Button
            variant="tertiary"
            className="mobile-preview-link"
            onClick={onPreview}
          >
            Explore development preview
            <IconArrowRight size={16} aria-hidden="true" />
          </Button>
        )}
      </div>
      <footer className="mobile-caption">
        <IconShieldLock size={16} aria-hidden="true" />
        <span>
          {auth
            ? "Sessions stay on this device until they expire or the app closes."
            : "Your tools and provider credentials stay on your execution hosts."}
        </span>
      </footer>
    </main>
  );
}
