import { useState } from "react";
import { McpHealthDetails } from "./McpHealthDetails";
import type { ManagedMcpDiagnostic } from "../types";
import {
  beginManagedMcpOAuth,
  completeManagedMcpOAuth,
  diagnoseManagedMcpServer,
  getManagedConfiguration,
  logoutManagedMcpOAuth,
  managedMcpOAuthStatus,
  saveSpaceConfiguration,
} from "../api";
import type { ManagedMcpOAuthLogin, ManagedMcpOAuthStatus } from "../types";
import { pluginConnectionRequest } from "../pluginConnection";

export function PluginMcpControls({
  spaceId,
  server,
  enabled,
  pluginActive,
  http,
  sessionRequired = false,
  onChanged,
}: {
  spaceId: string;
  server: string;
  enabled: boolean;
  pluginActive: boolean;
  http: boolean;
  sessionRequired?: boolean;
  onChanged?: (() => void) | undefined;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [diagnostic, setDiagnostic] = useState<ManagedMcpDiagnostic | null>(
    null,
  );
  const [status, setStatus] = useState<ManagedMcpOAuthStatus | null>(null);
  const [login, setLogin] = useState<ManagedMcpOAuthLogin | null>(null);
  const [callback, setCallback] = useState("");
  async function run(operation: () => Promise<void>) {
    if (busy || !enabled || !pluginActive) return;
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
  async function setEnabled(next: boolean) {
    if (busy) return;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const snapshot = await getManagedConfiguration();
      const request = pluginConnectionRequest(snapshot, spaceId, server, next);
      await saveSpaceConfiguration(request);
      setDiagnostic(null);
      setStatus(null);
      setMessage(
        next
          ? "Connection enabled. Test it after the runtime restarts."
          : "Connection disabled for this workspace.",
      );
      onChanged?.();
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : "Could not update this plugin connection.",
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
          disabled={busy || !pluginActive || (!enabled && sessionRequired)}
          onClick={() => void setEnabled(!enabled)}
        >
          {enabled ? "Disable connection" : "Enable all plugin tools"}
        </button>
        <button
          className="button secondary"
          disabled={busy || !enabled || !pluginActive}
          onClick={() =>
            void run(async () => {
              setDiagnostic(null);
              setMessage("");
              const result = await diagnoseManagedMcpServer(spaceId, server);
              setDiagnostic(result);
              setMessage(
                result.healthy
                  ? `${result.tools.length} allowlisted tools discovered.`
                  : (result.message ?? "Server diagnostic failed."),
              );
            })
          }
        >
          Test connection
        </button>
        {http && (
          <button
            className="button secondary"
            disabled={busy || !enabled || !pluginActive}
            onClick={() =>
              void run(async () => {
                setStatus(await managedMcpOAuthStatus(spaceId, server));
              })
            }
          >
            OAuth status
          </button>
        )}
      </div>
      {!pluginActive ? (
        <small>
          Activate this plugin digest before configuring its connection.
        </small>
      ) : sessionRequired ? (
        <small>
          Classic Outlook needs the Windows session connection below. Its
          sandboxed stdio connection cannot attach to the running Outlook
          process.
        </small>
      ) : !enabled ? (
        <small>
          A new connection permits every tool from this plugin, including tools
          added by a later update. Configure an exact tool list in plugin
          settings for narrower access.
        </small>
      ) : null}
      {busy && <p role="status">Updating or checking MCP connection…</p>}
      {diagnostic ? (
        <McpHealthDetails diagnostic={diagnostic} />
      ) : (
        message && <p role="status">{message}</p>
      )}
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
          disabled={busy || !enabled || !pluginActive}
          onClick={() =>
            void run(async () => {
              if (status.authenticated) {
                setStatus(await logoutManagedMcpOAuth(spaceId, server));
                setLogin(null);
              } else {
                setLogin(await beginManagedMcpOAuth(spaceId, server));
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
              disabled={busy || !enabled || !pluginActive}
            />
          </label>
          <button
            className="button primary"
            disabled={busy || !enabled || !pluginActive || !callback.trim()}
            onClick={() =>
              void run(async () => {
                setStatus(
                  await completeManagedMcpOAuth(spaceId, server, callback),
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
