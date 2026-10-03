import type { ManagedModelConfiguration } from "../../types";
import type { ImportedProviderSelection } from "./ImportedProviderPicker";

export function ImportedModelPicker({
  selection,
  selectedProfile,
  busy,
  onSelect,
}: {
  selection: ImportedProviderSelection;
  selectedProfile: string;
  busy: boolean;
  onSelect: (model: ManagedModelConfiguration) => void;
}) {
  return (
    <section className="imported-model-picker" aria-label="Imported models">
      <div className="imported-section-heading">
        <div>
          <h2>Models from {selection.provider.displayName}</h2>
          <p>Ready from your setup file. No connection is needed to choose.</p>
        </div>
      </div>
      <fieldset className="imported-model-list">
        <legend className="sr-only">Choose an imported model</legend>
        {selection.provider.models.map((model) => (
          <label key={model.profile} className="imported-model-option">
            <input
              type="radio"
              name="imported-model"
              aria-label={model.model}
              checked={selectedProfile === model.profile}
              disabled={busy}
              onChange={() => onSelect(model)}
            />
            <span>
              <strong>{model.model}</strong>
              <small>
                {model.contextWindowTokens.toLocaleString()} context ·{" "}
                {model.maxOutputTokens.toLocaleString()} output tokens
              </small>
              <small>
                {[
                  model.capabilities.toolCalls !== "off" && "Tools",
                  model.capabilities.streaming !== "off" && "Streaming",
                  model.capabilities.imageInputs !== "off" && "Images",
                  model.reasoningEffort && `${model.reasoningEffort} reasoning`,
                ]
                  .filter(Boolean)
                  .join(" · ") || "Text"}
              </small>
            </span>
            {selection.package.roles.primary === model.profile ? (
              <span className="setup-badge">Recommended</span>
            ) : null}
          </label>
        ))}
      </fieldset>
    </section>
  );
}
