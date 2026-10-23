import { useId, type ReactNode } from "react";
import { IconActivityHeartbeat, IconX } from "@tabler/icons-react";
import { Button, TextInput } from "./Controls.js";
import { DropdownSelect } from "./DropdownSelect.js";
import "../../styles/service-settings-editor.css";

export interface TelemetrySettingsDraft {
  resourceId: string | null;
  label: string;
  name: string;
  endpoint: string | null;
  protocol: "grpc" | "http_protobuf";
  timeoutMs: number;
  tracesEnabled: boolean;
  traceSampleRatioMillionths: number;
  metricsEnabled: boolean;
  metricExportIntervalMs: number;
  logsOtlp: boolean;
  logsStdoutJson: boolean;
  journalPayloads: "disabled" | "metadata" | "full";
  acknowledgeSensitiveContent: boolean;
  acknowledgeInsecureTransport: boolean;
  resourceAttributesText: string;
}

/** Hosts retain exporter lifecycle, destination review and explicit application. */
export function TelemetrySettingsEditor<T extends TelemetrySettingsDraft>({
  draft,
  busy,
  onChange,
  onSave,
  onCancel,
  saveDisabled = false,
  saveLabel = "Save telemetry connection",
  notice,
  allowSensitivePayloads = true,
  showDisplayLabel = true,
}: {
  draft: T;
  busy: boolean;
  onChange: (draft: T) => void;
  onSave: () => void;
  onCancel: () => void;
  saveDisabled?: boolean;
  saveLabel?: string;
  notice?: ReactNode;
  allowSensitivePayloads?: boolean;
  showDisplayLabel?: boolean;
}) {
  const id = useId();
  const sensitive = draft.journalPayloads === "full";
  const cannotSave =
    busy ||
    saveDisabled ||
    (sensitive &&
      (!allowSensitivePayloads || !draft.acknowledgeSensitiveContent));
  return (
    <form
      className="service-settings-editor"
      aria-labelledby={`${id}-heading`}
      onSubmit={(event) => {
        event.preventDefault();
        if (!cannotSave) onSave();
      }}
    >
      <header>
        <IconActivityHeartbeat size={18} aria-hidden="true" />
        <div>
          <h3 id={`${id}-heading`}>
            {draft.resourceId
              ? `Edit ${draft.label || "telemetry connection"}`
              : "Add telemetry connection"}
          </h3>
          <p>Choose a collector and the signals it receives.</p>
        </div>
        <Button
          type="button"
          variant="tertiary"
          disabled={busy}
          onClick={onCancel}
          aria-label="Close telemetry editor"
        >
          <IconX size={17} aria-hidden="true" />
        </Button>
      </header>
      {notice}
      <fieldset disabled={busy}>
        <legend>Identity</legend>
        <div className="service-settings-grid">
          {showDisplayLabel ? (
            <label>
              <span>Display label</span>
              <TextInput
                required
                value={draft.label}
                onChange={(event) =>
                  onChange({ ...draft, label: event.target.value })
                }
              />
            </label>
          ) : null}
          <label>
            <span>Service name</span>
            <TextInput
              required
              maxLength={128}
              value={draft.name}
              onChange={(event) =>
                onChange({ ...draft, name: event.target.value })
              }
            />
            <small>Emitted as the OpenTelemetry service identity.</small>
          </label>
        </div>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Collector destination</legend>
        <div className="service-settings-grid">
          <label>
            <span>Collector endpoint</span>
            <TextInput
              type="url"
              required={
                draft.tracesEnabled || draft.metricsEnabled || draft.logsOtlp
              }
              value={draft.endpoint ?? ""}
              placeholder="https://collector.example.com:4317"
              onChange={(event) =>
                onChange({ ...draft, endpoint: event.target.value || null })
              }
            />
          </label>
          <label>
            <span>Protocol</span>
            <DropdownSelect
              value={draft.protocol}
              onChange={(event) =>
                onChange({
                  ...draft,
                  protocol: event.target.value as T["protocol"],
                })
              }
            >
              <option value="grpc">OTLP gRPC</option>
              <option value="http_protobuf">OTLP HTTP/protobuf</option>
            </DropdownSelect>
          </label>
          <label>
            <span>Timeout (ms)</span>
            <TextInput
              type="number"
              required
              min={100}
              max={120_000}
              value={draft.timeoutMs}
              onChange={(event) =>
                onChange({ ...draft, timeoutMs: Number(event.target.value) })
              }
            />
          </label>
        </div>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Signals</legend>
        <div className="service-settings-grid">
          <section>
            <Toggle
              label="Traces"
              checked={draft.tracesEnabled}
              onChange={(tracesEnabled) =>
                onChange({ ...draft, tracesEnabled })
              }
            >
              Sample runtime operation traces.
            </Toggle>
            <label>
              <span>Trace sample (millionths)</span>
              <TextInput
                type="number"
                required
                min={0}
                max={1_000_000}
                disabled={!draft.tracesEnabled}
                value={draft.traceSampleRatioMillionths}
                onChange={(event) =>
                  onChange({
                    ...draft,
                    traceSampleRatioMillionths: Number(event.target.value),
                  })
                }
              />
              <small>
                {(draft.traceSampleRatioMillionths / 10_000).toLocaleString()}%
                of traces.
              </small>
            </label>
          </section>
          <section>
            <Toggle
              label="Metrics"
              checked={draft.metricsEnabled}
              onChange={(metricsEnabled) =>
                onChange({ ...draft, metricsEnabled })
              }
            >
              Export bounded runtime measurements.
            </Toggle>
            <label>
              <span>Metric interval (ms)</span>
              <TextInput
                type="number"
                required
                min={1_000}
                max={300_000}
                disabled={!draft.metricsEnabled}
                value={draft.metricExportIntervalMs}
                onChange={(event) =>
                  onChange({
                    ...draft,
                    metricExportIntervalMs: Number(event.target.value),
                  })
                }
              />
            </label>
          </section>
          <section>
            <Toggle
              label="OTLP logs"
              checked={draft.logsOtlp}
              onChange={(logsOtlp) => onChange({ ...draft, logsOtlp })}
            >
              Send structured runtime logs.
            </Toggle>
            <Toggle
              label="JSON console logs"
              checked={draft.logsStdoutJson}
              onChange={(logsStdoutJson) =>
                onChange({ ...draft, logsStdoutJson })
              }
            >
              Also emit structured logs through the host console.
            </Toggle>
          </section>
        </div>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Audit content and transport</legend>
        <label>
          <span>Audit content</span>
          <DropdownSelect
            value={draft.journalPayloads}
            onChange={(event) =>
              onChange({
                ...draft,
                journalPayloads: event.target.value as T["journalPayloads"],
                acknowledgeSensitiveContent:
                  event.target.value === "full" &&
                  draft.acknowledgeSensitiveContent,
              })
            }
          >
            <option value="disabled">Disabled</option>
            <option value="metadata">Metadata only</option>
            {allowSensitivePayloads ? (
              <option value="full">Full sensitive payloads</option>
            ) : null}
          </DropdownSelect>
        </label>
        {!allowSensitivePayloads ? (
          <p>
            Instance administrators cannot export other users’ private message
            or tool content. Only content-free metadata is available.
          </p>
        ) : null}
        {sensitive && allowSensitivePayloads ? (
          <Toggle
            label="Acknowledge sensitive journal content"
            checked={draft.acknowledgeSensitiveContent}
            onChange={(acknowledgeSensitiveContent) =>
              onChange({ ...draft, acknowledgeSensitiveContent })
            }
          >
            Full audit content can include prompts, model output, tool
            arguments, results and personal information. Confirm that the
            collector is authorized to receive it.
          </Toggle>
        ) : null}
        <Toggle
          label="Allow insecure remote transport"
          checked={draft.acknowledgeInsecureTransport}
          onChange={(acknowledgeInsecureTransport) =>
            onChange({ ...draft, acknowledgeInsecureTransport })
          }
        >
          Allow unencrypted remote export only when the deployment permits it.
        </Toggle>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Resource attributes</legend>
        <label>
          <span>Attributes</span>
          <textarea
            rows={4}
            maxLength={16_384}
            value={draft.resourceAttributesText}
            placeholder="deployment.environment=production"
            onChange={(event) =>
              onChange({ ...draft, resourceAttributesText: event.target.value })
            }
          />
          <small>
            One key=value per line; at most 32 bounded attributes. Omit secrets
            and personal identifiers.
          </small>
        </label>
      </fieldset>
      <footer>
        <Button type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" disabled={cannotSave}>
          {saveLabel}
        </Button>
      </footer>
    </form>
  );
}

function Toggle({
  label,
  checked,
  onChange,
  children,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  children: ReactNode;
}) {
  return (
    <label className="service-settings-toggle">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span>
        <strong>{label}</strong>
        <small>{children}</small>
      </span>
    </label>
  );
}
