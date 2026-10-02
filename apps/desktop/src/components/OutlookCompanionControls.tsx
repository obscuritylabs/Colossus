import { useEffect, useState } from "react";
import {
  configureOutlookCompanion,
  diagnoseManagedMcpServer,
  getOutlookCompanionStatus,
} from "../api";
import type { OutlookCompanionStatus } from "../plugins";

export function OutlookCompanionControls({
  spaceId,
  pluginActive,
  onChanged,
}: {
  spaceId: string;
  pluginActive: boolean;
  onChanged?: (() => void) | undefined;
}) {
  const [status, setStatus] = useState<OutlookCompanionStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [check, setCheck] = useState("");

  useEffect(() => {
    let current = true;
    void getOutlookCompanionStatus(spaceId)
      .then((value) => {
        if (current) setStatus(value);
      })
      .catch((reason: unknown) => {
        if (current)
          setError(
            reason instanceof Error
              ? reason.message
              : "Could not read Outlook status.",
          );
      });
    return () => {
      current = false;
    };
  }, [spaceId]);

  if (!status) return error ? <p role="alert">{error}</p> : null;
  if (!status.supported) return null;

  async function toggle() {
    if (!status || busy) return;
    setBusy(true);
    setError("");
    setCheck("");
    try {
      const result = await configureOutlookCompanion(spaceId, !status.enabled);
      setStatus(result);
      onChanged?.();
    } catch (reason) {
      setError(
        reason instanceof Error
          ? reason.message
          : "Could not change the Outlook session connection.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function testConnection() {
    if (!status?.activeDigest || busy) return;
    setBusy(true);
    setError("");
    setCheck("");
    try {
      const result = await diagnoseManagedMcpServer(spaceId, "outlook-session");
      setCheck(
        result.healthy
          ? `${result.tools.length} allowlisted Outlook tools discovered.`
          : (result.message ?? "Outlook session connection failed."),
      );
    } catch (reason) {
      setError(
        reason instanceof Error
          ? reason.message
          : "Could not test Outlook connection.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="plugin-server" aria-label="Classic Outlook session">
      <h5>Classic Outlook session</h5>
      <p>
        Run the signed Outlook plugin in your Windows sign-in session. This
        connection can read and change mail through its exact listed tools;
        Colossus still reviews and audits tool calls.
      </p>
      <button
        type="button"
        className="button secondary"
        disabled={busy || (!pluginActive && !status.enabled)}
        onClick={() => void toggle()}
      >
        {status.enabled
          ? "Disconnect Outlook session"
          : "Connect Outlook session"}
      </button>
      <button
        type="button"
        className="button secondary"
        disabled={busy || !status.activeDigest}
        onClick={() => void testConnection()}
      >
        Test connection
      </button>
      {status.enabled && (
        <p role="status">
          {status.activeDigest
            ? `Helper running from verified digest ${status.activeDigest}.`
            : "Enabled; start this Workspace to connect."}
        </p>
      )}
      {check && <p role="status">{check}</p>}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
