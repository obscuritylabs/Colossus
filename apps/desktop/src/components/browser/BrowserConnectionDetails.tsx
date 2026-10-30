/** @jsxRuntime classic */
/** @jsx element */
import { element } from "./browser-jsx";
import { useEffect, useRef } from "react";
import { Button } from "@colossus/ui/components/Controls";
import type { BrowserDiagnostics, BrowserEngine } from "../../browser-api";

export function nativeNavigationError(code: number | null | undefined): string {
  if (code === null || code === undefined) return "None reported";
  const labels: Record<number, string> = {
    1: "Certificate hostname mismatch",
    2: "Certificate expired",
    3: "Client certificate rejected",
    4: "Certificate revoked",
    5: "Certificate invalid",
    6: "Server unreachable",
    7: "Connection timed out",
    8: "Invalid server response",
    9: "Connection aborted",
    10: "Connection reset",
    11: "Network disconnected",
    12: "Could not connect",
    13: "Hostname could not be resolved",
    14: "Navigation cancelled",
    15: "Redirect failed",
    16: "Unexpected engine failure",
    17: "Sign-in credentials required",
    18: "Proxy authentication required",
  };
  return `${labels[code] ?? "Engine error"} (code ${code})`;
}

export function BrowserConnectionDetails({
  details,
  engineKind,
  onClose,
}: {
  details: BrowserDiagnostics | undefined;
  engineKind?: BrowserEngine["kind"] | undefined;
  onClose: () => void;
}) {
  const close = useRef<HTMLButtonElement>(null);
  const webview2 = details?.systemEngine && engineKind !== "webkit";
  useEffect(() => {
    close.current?.focus();
  }, []);
  return (
    <div
      className="browser-pki browser-connection-details"
      role="dialog"
      aria-modal="false"
      aria-labelledby="browser-connection-title"
      data-browser-occluded
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="browser-pki-bar">
        <h3 id="browser-connection-title">Connection details</h3>
        <Button ref={close} onClick={onClose}>
          Close
        </Button>
      </div>
      <dl>
        <dt>Origin</dt>
        <dd>{details?.origin ?? "No HTTP(S) page"}</dd>
        <dt>Human profile</dt>
        <dd>
          {engineKind === "webkit"
            ? "WebKit"
            : details?.systemEngine
              ? "Windows WebView2"
              : "Supervised Chromium"}{" "}
          · {details?.persistent ? "Saved sign-ins" : "Temporary session"}
        </dd>
        <dt>User-Agent</dt>
        <dd>
          {details?.customUserAgent
            ? "Custom native value applied"
            : "Engine default"}
        </dd>
        <dt>HTTP response</dt>
        <dd>
          {details?.httpStatus
            ? `HTTP ${details.httpStatus}`
            : "Not reported by this engine"}
        </dd>
        <dt>Redirects</dt>
        <dd>{webview2 ? details.redirects : "Not reported by this engine"}</dd>
        <dt>Client certificate</dt>
        <dd>
          {details?.clientCertificateRequested
            ? webview2
              ? "Requested · Windows certificate chooser"
              : engineKind === "webkit"
                ? "Requested · native operating-system identity"
                : "Requested · native exact-origin review"
            : "No observed challenge"}
        </dd>
        <dt>Certificate validation</dt>
        <dd>
          {details?.tlsFailed
            ? "Certificate rejected · no bypass"
            : "Enabled · native trust policy"}
        </dd>
        <dt>Native navigation error</dt>
        <dd>{nativeNavigationError(details?.nativeError)}</dd>
      </dl>
      {details?.httpStatus === 401 || details?.httpStatus === 403 ? (
        <p role="status">
          The server denied access. Check that you chose the intended account or
          client certificate, then check the site’s sign-in and embedded-browser
          policy. A User-Agent override cannot guarantee access.
        </p>
      ) : null}
      <p>
        Only origin and categorical engine metadata appear here. Cookies,
        authorization headers, certificate keys, URL paths and sign-in codes are
        never included.
      </p>
      <p>
        Saved human sign-ins are separate from agent sessions. Manage cookies
        and the User-Agent in Settings → Browser.
      </p>
    </div>
  );
}
