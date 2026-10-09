import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import type {
  BrowserCertificateAction,
  BrowserCertificateStatus,
} from "../../browser-api";
import { browserErrorMessage } from "../../browser-api";
import { useAppearance } from "../../theme/AppearanceProvider";
import type { BrowserController } from "./useBrowser";

export function BrowserCertificates({
  controller,
  onClose,
}: {
  controller: BrowserController;
  onClose: () => void;
}) {
  const preferences = useAppearance();
  const [status, setStatus] = useState<BrowserCertificateStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const closeRef = useRef<HTMLButtonElement>(null);
  const operation = useRef(0);
  async function perform(action: BrowserCertificateAction) {
    const sequence = ++operation.current;
    setBusy(true);
    setError("");
    try {
      const status = await controller.certificates(action, {
        colorScheme: preferences.resolvedColorTheme,
        textSize: preferences.textSize,
      });
      if (sequence === operation.current) setStatus(status);
    } catch (cause) {
      if (sequence === operation.current) setError(browserErrorMessage(cause));
    } finally {
      if (sequence === operation.current) setBusy(false);
    }
  }
  useEffect(() => {
    closeRef.current?.focus();
    void perform("status");
    return () => {
      ++operation.current;
    };
    // Scope and controller generation are bound in the host request, not derived here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div
      className="browser-certificates"
      role="dialog"
      aria-modal="false"
      aria-labelledby="browser-certificates-title"
      data-browser-occluded
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="browser-certificate-heading">
        <h3 id="browser-certificates-title">Browser certificates</h3>
        <Button ref={closeRef} onClick={onClose}>
          Close
        </Button>
      </div>
      <p>{status?.message ?? "Reading native certificate support…"}</p>
      {status ? (
        <dl>
          <dt>Trust scope</dt>
          <dd>
            {status.scope === "operating_system_user"
              ? "Operating-system user store"
              : "Unavailable"}
          </dd>
          <dt>Client certificate selection</dt>
          <dd>
            {status.clientIdentitySelectionReady ? "Available" : "Unavailable"}
          </dd>
          {status.acceptancePending ? (
            <>
              <dt>Native acceptance</dt>
              <dd>Pending</dd>
            </>
          ) : null}
        </dl>
      ) : null}
      {error ? <p role="alert">{error}</p> : null}
      {status?.fingerprintsSha256.length ? (
        <div role="status">
          <p>Imported certificate fingerprints (SHA-256)</p>
          <ul>
            {status.fingerprintsSha256.map((fingerprint) => (
              <li key={fingerprint}>
                <code>{fingerprint}</code>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <div className="browser-certificate-actions">
        <Button
          disabled={busy || !status?.caImportAvailable}
          onClick={() => void perform("import_ca")}
        >
          Import CA certificate
        </Button>
        <Button
          disabled={busy || !status?.pfxImportAvailable}
          onClick={() => void perform("import_client_identity")}
        >
          Import PKCS#12 identity
        </Button>
      </div>
      <p>
        Files and passphrases are selected in native dialogs. Imports can affect
        other applications. Client certificate import does not grant an agent
        access.
      </p>
    </div>
  );
}
