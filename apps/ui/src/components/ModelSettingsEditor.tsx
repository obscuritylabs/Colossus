import type { ReactNode } from "react";
import { IconCheck, IconCpu, IconX } from "@tabler/icons-react";
import { Button, TextInput } from "./Controls";
import { DropdownSelect } from "./DropdownSelect";
import {
  ModelFeatureControl,
  type ModelFeatureMode,
} from "./ModelFeatureControl";
import { ProviderModelPicker } from "./ProviderModelPicker";
import type { ProviderCatalogModel } from "../lib/provider-catalog";
import "../../styles/model-settings-editor.css";

export type ModelSettingsReasoningEffort =
  "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra";
export interface ModelSettingsDraft {
  resourceId: string | null;
  label: string;
  profile: string;
  providerProfile: string;
  model: string;
  contextWindowTokens: number;
  maxOutputTokens: number;
  toolCalls: ModelFeatureMode;
  streaming: ModelFeatureMode;
  imageInputs: ModelFeatureMode;
  serverCompaction?: ModelFeatureMode;
  declared?:
    | {
        toolCalls?: boolean | null;
        streaming?: boolean | null;
        imageInputs?: boolean | null;
        serverCompaction?: boolean | null;
      }
    | undefined;
  reasoningEffort: ModelSettingsReasoningEffort | null;
}

/** The complete Desktop model settings surface; hosts retain catalog transport and mutations. */
export function ModelSettingsEditor<T extends ModelSettingsDraft>({
  draft,
  providers,
  busy,
  onChange,
  onCancel,
  onSave,
  connectionKey,
  catalogDisabled = false,
  catalogNotice,
  onLoadModels,
  onSelectCatalogModel,
  onProviderChange,
  onModelIdentifierChange,
  providerIcon,
}: {
  draft: T;
  providers: readonly { profile: string; label: string }[];
  busy: boolean;
  onChange: (draft: T) => void;
  onCancel: () => void;
  onSave: () => void;
  connectionKey: string;
  catalogDisabled?: boolean;
  catalogNotice?: ReactNode;
  onLoadModels: () => Promise<ProviderCatalogModel[]>;
  onSelectCatalogModel: (model: ProviderCatalogModel) => void;
  onProviderChange: (profile: string) => void;
  onModelIdentifierChange: (model: string) => void;
  providerIcon?: (profile: string) => ReactNode;
}) {
  const title = draft.resourceId
    ? `Edit ${draft.label || "model"}`
    : "Add model";
  return (
    <form
      className="model-settings-editor model-editor"
      aria-labelledby="model-editor-heading"
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy) onSave();
      }}
    >
      <div className="model-editor-heading">
        <span className="model-editor-icon">
          <IconCpu size={18} aria-hidden="true" />
        </span>
        <div>
          <h4 id="model-editor-heading">{title}</h4>
          <p>Add a model and describe the limits and features it supports.</p>
        </div>
        <Button
          className="model-editor-close"
          variant="tertiary"
          type="button"
          aria-label="Close model editor"
          disabled={busy}
          onClick={onCancel}
        >
          <IconX size={17} />
        </Button>
      </div>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-identity-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-identity-heading">Identity</h5>
          <p>
            Choose a name shown in settings and an ID used in configuration.
          </p>
        </div>
        <div className="model-editor-grid model-identity-grid">
          <label>
            <span>Display label</span>
            <TextInput
              disabled={busy}
              value={draft.label}
              placeholder="For example, Primary reasoning model"
              aria-describedby="model-label-help"
              required
              onChange={(event) =>
                onChange({ ...draft, label: event.target.value })
              }
            />
            <small id="model-label-help">Shown to people in settings.</small>
          </label>
          <label>
            <span>Profile ID</span>
            <TextInput
              disabled={busy}
              value={draft.profile}
              placeholder="primary-reasoning"
              aria-describedby="model-profile-help"
              required
              onChange={(event) =>
                onChange({ ...draft, profile: event.target.value })
              }
            />
            <small id="model-profile-help">
              Used to identify this model in workspace configuration.
            </small>
          </label>
        </div>
      </section>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-route-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-route-heading">Provider connection</h5>
          <p>Choose the provider and enter its model ID.</p>
        </div>
        <div className="model-editor-grid model-route-grid">
          <label>
            <span>Provider profile</span>
            <DropdownSelect
              disabled={busy}
              value={draft.providerProfile}
              aria-describedby="model-provider-help"
              {...(providerIcon
                ? { renderOptionIcon: (option) => providerIcon(option.value) }
                : {})}
              required
              onChange={(event) => onProviderChange(event.target.value)}
            >
              <option value="">Select provider</option>
              {providers.map((provider) => (
                <option key={provider.profile} value={provider.profile}>
                  {provider.label}
                </option>
              ))}
            </DropdownSelect>
            <small id="model-provider-help">
              Uses one of the provider connections configured above.
            </small>
          </label>
          <label>
            <span>Model identifier</span>
            <TextInput
              disabled={busy}
              value={draft.model}
              placeholder="Provider-specific model name"
              aria-describedby="model-identifier-help"
              required
              onChange={(event) => onModelIdentifierChange(event.target.value)}
            />
            <small id="model-identifier-help">
              Enter the model name expected by the selected provider.
            </small>
          </label>
        </div>
        <ProviderModelPicker
          key={connectionKey}
          connectionKey={connectionKey}
          model={draft.model}
          disabled={busy || catalogDisabled}
          onLoad={onLoadModels}
          onSelect={onSelectCatalogModel}
        />
        {catalogNotice}
      </section>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-budget-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-budget-heading">Token limits</h5>
          <p>
            Set how much context the model can read and how long it can reply.
          </p>
        </div>
        <div className="model-editor-grid model-budget-grid">
          <label>
            <span>Context window (tokens)</span>
            <TextInput
              disabled={busy}
              type="number"
              min={1_024}
              value={draft.contextWindowTokens}
              aria-describedby="model-context-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  contextWindowTokens: Number(event.target.value),
                })
              }
            />
            <small id="model-context-help">
              Minimum 1,024 tokens, including input and output.
            </small>
          </label>
          <label>
            <span>Maximum output (tokens)</span>
            <TextInput
              disabled={busy}
              type="number"
              min={1}
              value={draft.maxOutputTokens}
              aria-describedby="model-output-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  maxOutputTokens: Number(event.target.value),
                })
              }
            />
            <small id="model-output-help">
              Upper bound for one generated model response.
            </small>
          </label>
        </div>
      </section>
      <section
        className="model-editor-section"
        aria-labelledby="model-editor-capabilities-heading"
      >
        <div className="model-editor-section-heading">
          <h5 id="model-editor-capabilities-heading">Supported features</h5>
          <p>
            Auto uses advertised support and provider responses. On forces an
            attempt; Off disables the feature.
          </p>
        </div>
        <div className="model-capability-grid">
          {(
            [
              ["toolCalls", "Tool calls", "Let the model request tools."],
              ["streaming", "Streaming", "Show response text as it arrives."],
              ["imageInputs", "Image inputs", "Receive attached images."],
              [
                "serverCompaction",
                "Server compaction",
                "Use Responses compaction with local summarization as fallback.",
              ],
            ] as const
          ).map(([feature, label, description]) => (
            <ModelFeatureControl
              key={feature}
              label={label}
              declared={draft.declared?.[feature]}
              description={description}
              value={draft[feature] ?? "auto"}
              disabled={busy}
              onChange={(mode) => onChange({ ...draft, [feature]: mode })}
            />
          ))}
        </div>
        <div className="model-reasoning-control">
          <label>
            <span>Reasoning effort</span>
            <DropdownSelect
              disabled={busy}
              value={draft.reasoningEffort ?? "inherit"}
              aria-describedby="model-reasoning-help"
              onChange={(event) =>
                onChange({
                  ...draft,
                  reasoningEffort:
                    event.target.value === "inherit"
                      ? null
                      : (event.target.value as ModelSettingsReasoningEffort),
                })
              }
            >
              <option value="inherit">Provider default</option>
              {[
                "none",
                "minimal",
                "low",
                "medium",
                "high",
                "xhigh",
                "max",
                "ultra",
              ].map((effort) => (
                <option key={effort} value={effort}>
                  {effort}
                </option>
              ))}
            </DropdownSelect>
            <small id="model-reasoning-help">
              Leave at provider default unless the provider supports a specific
              effort level.
            </small>
          </label>
        </div>
      </section>
      <div className="model-editor-actions">
        <Button
          variant="secondary"
          type="button"
          disabled={busy}
          onClick={onCancel}
        >
          Cancel
        </Button>
        <Button variant="primary" type="submit" disabled={busy}>
          <IconCheck size={16} />
          {draft.resourceId ? "Save changes" : "Add model"}
        </Button>
      </div>
    </form>
  );
}
