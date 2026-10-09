/** @jsxRuntime classic */
/** @jsx element */
// The classic JSX transform keeps this bounded browser surface compact.
import { element } from "./browser-jsx";
import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui/components/Controls";
import type {
  BrowserCertificateAction,
  BrowserCertificateStatus,
} from "../../browser-api";
import { browserErrorMessage } from "../../browser-api";
import type { BrowserController } from "./useBrowser";

export function BrowserCertificates({
  controller,
  onClose,
}: {
  controller: BrowserController;
  onClose: () => void;
}) {
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
      const status = await controller.certificates(action);
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
      className="browser-pki"
      role="dialog"
      aria-modal="false"
      aria-labelledby="browser-pki-title"
      data-browser-occluded
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="browser-pki-bar">
        <h3 id="browser-pki-title">Browser certificates</h3>
        <Button ref={closeRef} onClick={onClose}>
          Close
        </Button>
      </div>
      <p role={error ? "alert" : "status"}>
        {error || status?.message || "Loading certificates…"}
      </p>
      {status ? (
        <p>
          Trust scope:{" "}
          {status.scope === "operating_system_user"
            ? "Operating-system user store"
            : "Unavailable"}
          . Client selection:{" "}
          {status.clientIdentitySelectionReady ? "Available" : "Unavailable"}.{" "}
          {status.acceptancePending ? "Native acceptance pending." : ""}
        </p>
      ) : null}
      {status?.fingerprintsSha256.length ? (
        <div role="status">
          <p>Fingerprints (SHA-256)</p>
          <pre>{status.fingerprintsSha256.join("\n")}</pre>
        </div>
      ) : null}
      <div className="browser-pki-bar">
        {(
          [
            ["import_ca", "Import CA certificate", status?.caImportAvailable],
            [
              "import_client_identity",
              "Import PKCS#12 identity",
              status?.pfxImportAvailable,
            ],
          ] as const
        ).map(([action, label, available]) => (
          <Button
            key={action}
            disabled={busy || !available}
            onClick={() => void perform(action)}
          >
            {label}
          </Button>
        ))}
      </div>
      <p>
        Native dialogs handle files and passphrases. Imports affect other apps
        and do not authorize agents.
      </p>
    </div>
  );
}
