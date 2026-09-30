import type { ReactNode } from "react";
import {
  IconActivityHeartbeat,
  IconAlertTriangle,
  IconCircleCheck,
  IconCloud,
  IconCpu,
} from "@tabler/icons-react";
import type { ManagedRuntimeDiagnostic } from "../types";
import "./runtime-profile-row.css";

export function RuntimeProfileRow({
  kind,
  label,
  subtitle,
  selected,
  busy,
  testing,
  disabledReason,
  diagnostic,
  error,
  onTest,
  children,
}: {
  kind: "provider" | "model";
  label: string;
  subtitle: string;
  selected: boolean;
  busy: boolean;
  testing: boolean;
  disabledReason: string | null;
  diagnostic: ManagedRuntimeDiagnostic | undefined;
  error: string | undefined;
  onTest: () => void;
  children: ReactNode;
}) {
  const result = disabledReason ? undefined : diagnostic;
  const failure = disabledReason ? undefined : error;
  const status = testing
    ? "Testing…"
    : failure || result?.ready === false
      ? "Failed"
      : result?.ready
        ? "Passed"
        : selected
          ? "Selected"
          : "Available";
  return (
    <div
      className="managed-profile-resource"
      role="group"
      aria-label={`${kind === "provider" ? "Provider" : "Model"} ${label}`}
    >
      <div className="managed-list-row mcp-diagnostic-row">
        <span className="resource-icon">
          {kind === "provider" ? (
            <IconCloud size={18} />
          ) : (
            <IconCpu size={18} />
          )}
        </span>
        <div>
          <strong>{label}</strong>
          <small>{subtitle}</small>
          {selected && disabledReason ? <small>{disabledReason}</small> : null}
        </div>
        <span
          className={`status-chip ${failure || result?.ready === false ? "tone-danger" : result?.ready ? "tone-success" : "tone-neutral"}`}
        >
          {status}
        </span>
        <div className="resource-actions">
          <button
            type="button"
            className="button secondary"
            aria-label={`Test ${kind} ${label}`}
            disabled={busy || !!disabledReason}
            title={disabledReason ?? `Test ${kind} ${label}`}
            onClick={onTest}
          >
            <IconActivityHeartbeat size={15} aria-hidden="true" />
            {testing ? "Testing…" : "Test"}
          </button>
        </div>
        {children}
      </div>
      {testing ? (
        <div className="profile-diagnostic-progress" role="status">
          Testing {label}…
        </div>
      ) : failure || result ? (
        <div className="mcp-diagnostic-detail profile-diagnostic-detail">
          <div className="mcp-health-summary">
            {result?.ready ? (
              <IconCircleCheck
                size={20}
                className="is-healthy"
                aria-hidden="true"
              />
            ) : (
              <IconAlertTriangle
                size={20}
                className="is-failed"
                aria-hidden="true"
              />
            )}
            <div>
              <strong>
                {failure
                  ? "Test could not complete"
                  : result?.ready
                    ? "Test passed"
                    : "Test failed"}
              </strong>
              <p role={failure || !result?.ready ? "alert" : "status"}>
                {failure ??
                  (result?.ready
                    ? `${kind === "provider" ? "Provider" : "Model"} checks completed successfully.`
                    : "Review the failed checks below, update the connection, and retry.")}
              </p>
            </div>
          </div>
          {result?.checks.length ? (
            <details className="mcp-health-disclosure" open={!result.ready}>
              <summary>Test details</summary>
              <ul
                className="profile-diagnostic-checks"
                aria-label={`${label} test details`}
                tabIndex={0}
              >
                {result.checks.map((check, index) => (
                  <li key={`${check.name}-${index}`}>
                    <div>
                      <strong>{check.name.replaceAll("_", " ")}</strong>
                      <span>{check.status.replaceAll("_", " ")}</span>
                    </div>
                    <p>{check.detail}</p>
                  </li>
                ))}
              </ul>
            </details>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
