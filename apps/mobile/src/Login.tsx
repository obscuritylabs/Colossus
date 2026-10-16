import { useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import {
  IconArrowLeft,
  IconArrowUpRight,
  IconLock,
  IconServer,
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
  const [busy, setBusy] = useState(false),
    [error, setError] = useState("");
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
        /* Saving an address is optional. */
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
    try {
      const identity = await client.login(username.trim(), entered);
      if (auth) onLogin(identity, origin, auth);
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="mobile-login">
      {client.preview && (
        <p className="mobile-preview-banner" role="status">
          Development preview · Sample data
        </p>
      )}
      <header className="mobile-brand">
        <img src={mark} alt="" />
        <strong>Colossus</strong>
      </header>
      {auth?.classification?.enabled && (
        <div className="mobile-classification">{auth.classification.text}</div>
      )}
      <div className="mobile-login-content">
        {auth && (
          <Button
            variant="tertiary"
            disabled={busy}
            onClick={() => {
              setAuth(null);
              setPassword("");
              setError("");
            }}
          >
            <IconArrowLeft size={18} /> Change server
          </Button>
        )}
        <div className="mobile-login-symbol">
          {auth ? <IconLock size={28} /> : <IconServer size={28} />}
        </div>
        <h1>
          {auth ? "Sign in to Colossus" : "Your agents. Wherever you are."}
        </h1>
        <p className="mobile-muted">
          {auth
            ? "Use your organization’s Colossus account."
            : "Connect to your organization’s Control Plane to continue work across your hosts."}
        </p>
        {notice && (
          <p role="status" className="mobile-notice">
            {notice}
          </p>
        )}
        {error && (
          <p role="alert" className="mobile-error">
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
              id="server"
              type="url"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              autoComplete="url"
              placeholder="https://colossus.example.com"
              value={origin}
              disabled={busy}
              onChange={(event) => setOrigin(event.target.value)}
              required
            />
            <p className="mobile-caption">
              Use the address provided by your organization.
            </p>
            <Button
              type="submit"
              variant="primary"
              disabled={busy || !origin.trim()}
            >
              {busy ? "Connecting…" : "Continue"}
            </Button>
          </form>
        ) : (
          <>
            <p className="mobile-server">
              <IconLock size={16} />
              <span>{origin}</span>
            </p>
            {auth.local_enabled && (
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  void signIn();
                }}
              >
                <label htmlFor="username">Username</label>
                <TextInput
                  id="username"
                  autoComplete="username"
                  autoCapitalize="none"
                  autoCorrect="off"
                  value={username}
                  disabled={busy}
                  onChange={(event) => setUsername(event.target.value)}
                  required
                />
                <label htmlFor="password">Password</label>
                <TextInput
                  id="password"
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  disabled={busy}
                  onChange={(event) => setPassword(event.target.value)}
                  required
                />
                <Button
                  type="submit"
                  variant="primary"
                  disabled={busy || !username.trim() || !password}
                >
                  {busy ? "Signing in…" : "Sign in"}
                </Button>
                <p className="mobile-caption">
                  Your session stays in this app until it expires or the app
                  closes.
                </p>
              </form>
            )}
            {auth.oidc && (
              <section className="mobile-sso">
                <h2>Organization sign-in</h2>
                <p className="mobile-muted">
                  Continue with {auth.oidc.label} in your browser. Browser
                  sign-in stays in Control Plane; it does not sign in this app
                  yet.
                </p>
                <Button
                  onClick={() => {
                    void client
                      .openBrowser()
                      .catch((failure) => setError(messageOf(failure)));
                  }}
                >
                  <IconArrowUpRight size={18} /> Open Control Plane
                </Button>
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
        {onPreview && (
          <Button
            variant="tertiary"
            className="mobile-preview-link"
            onClick={onPreview}
          >
            Explore development preview
          </Button>
        )}
      </div>
      <footer className="mobile-caption">
        Your tools and provider credentials stay on your execution hosts.
      </footer>
    </main>
  );
}
