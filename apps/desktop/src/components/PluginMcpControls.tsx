import { useState } from "react";
import {
  beginManagedMcpOAuth,
  completeManagedMcpOAuth,
  diagnoseManagedMcpServer,
  logoutManagedMcpOAuth,
  managedMcpOAuthStatus,
} from "../api";
import type { ManagedMcpOAuthLogin, ManagedMcpOAuthStatus } from "../types";

/** Explicit diagnostics and OAuth only; this component cannot enable a server. */
export function PluginMcpControls({
  targetId,
  server,
  enabled,
  http,
}: {
  targetId: string;
  server: string;
  enabled: boolean;
  http: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [status, setStatus] = useState<ManagedMcpOAuthStatus | null>(null);
  const [login, setLogin] = useState<ManagedMcpOAuthLogin | null>(null);
  const [callback, setCallback] = useState("");
  async function run(operation: () => Promise<void>) {
    if (busy || !enabled) return;
    setBusy(true);
    setError("");
    try {
      await operation();
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : "MCP operation failed. Check the applied server configuration.",
      );
    } finally {
      setBusy(false);
    }
  }
  return (
    <div role="group" aria-label={`${server} connection`}>
      <div className="plugin-actions">
        <button
          className="button secondary"
          disabled={busy || !enabled}
          onClick={() =>
            void run(async () => {
              const result = await diagnoseManagedMcpServer(targetId, server);
              setMessage(
                result.healthy
                  ? `${result.tools.length} allowlisted tools discovered.`
                  : "Server diagnostic failed.",
              );
            })
          }
        >
          Test connection
        </button>
        {http && (
          <button
            className="button secondary"
            disabled={busy || !enabled}
            onClick={() =>
              void run(async () => {
                setStatus(await managedMcpOAuthStatus(targetId, server));
              })
            }
          >
            OAuth status
          </button>
        )}
      </div>
      {!enabled && (
        <small>
          Enable and apply this server explicitly in plugin settings first.
        </small>
      )}
      {busy && <p role="status">Checking MCP connection…</p>}
      {message && <p role="status">{message}</p>}
      {error && <p role="alert">{error}</p>}
      {status && (
        <p>
          {!status.configured
            ? "No OAuth overlay configured."
            : status.authenticated
              ? "Signed in"
              : "Signed out"}
        </p>
      )}
      {status?.configured && (
        <button
          className="button secondary"
          disabled={busy || !enabled}
          onClick={() =>
            void run(async () => {
              if (status.authenticated) {
                setStatus(await logoutManagedMcpOAuth(targetId, server));
                setLogin(null);
              } else {
                setLogin(await beginManagedMcpOAuth(targetId, server));
              }
            })
          }
        >
          {status.authenticated ? "Sign out" : "Sign in"}
        </button>
      )}
      {login && (
        <div className="plugin-actions">
          <a
            className="button secondary"
            href={login.authorizationUrl}
            target="_blank"
            rel="noreferrer"
          >
            Open authorization
          </a>
          <label>
            OAuth callback URL
            <input
              type="url"
              value={callback}
              placeholder={login.callbackUrl}
              onChange={(event) => setCallback(event.target.value)}
              disabled={busy || !enabled}
            />
          </label>
          <button
            className="button primary"
            disabled={busy || !enabled || !callback.trim()}
            onClick={() =>
              void run(async () => {
                setStatus(
                  await completeManagedMcpOAuth(targetId, server, callback),
                );
                setLogin(null);
                setCallback("");
              })
            }
          >
            Complete sign-in
          </button>
        </div>
      )}
    </div>
  );
}
