import { useEffect, useId, useRef, useState } from "react";
import { IconLoader2 } from "@tabler/icons-react";
import {
  filterCatalogModels,
  type ProviderCatalogModel,
} from "../providerCatalog";
import "./provider-catalog.css";

export function ProviderModelPicker({
  connectionKey,
  model,
  disabled = false,
  autoLoad = false,
  onLoad,
  onSelect,
}: {
  connectionKey: string;
  model: string;
  disabled?: boolean;
  autoLoad?: boolean;
  onLoad: () => Promise<ProviderCatalogModel[]>;
  onSelect: (model: ProviderCatalogModel) => void;
}) {
  const id = useId();
  const [models, setModels] = useState<ProviderCatalogModel[]>([]);
  const [query, setQuery] = useState("");
  const [state, setState] = useState<"idle" | "loading" | "loaded" | "error">(
    "idle",
  );
  const [error, setError] = useState("");
  const generation = useRef(0);
  const loadRef = useRef(onLoad);
  loadRef.current = onLoad;
  const loadingRef = useRef(false);

  async function load() {
    if (loadingRef.current) return;
    const request = ++generation.current;
    loadingRef.current = true;
    setState("loading");
    setError("");
    try {
      const result = await loadRef.current();
      if (request !== generation.current) return;
      setModels(result);
      setState("loaded");
    } catch (reason) {
      if (request !== generation.current) return;
      setState("error");
      setError(
        reason instanceof Error
          ? reason.message
          : "Could not load models from this provider.",
      );
    } finally {
      if (request === generation.current) loadingRef.current = false;
    }
  }

  useEffect(() => {
    generation.current += 1;
    loadingRef.current = false;
    setModels([]);
    setQuery("");
    setState("idle");
    setError("");
    if (autoLoad && !disabled) void load();
    return () => {
      generation.current += 1;
      loadingRef.current = false;
    };
  }, [connectionKey, autoLoad]);

  const filtered = filterCatalogModels(models, query);
  const selected = models.find((candidate) => candidate.id === model);
  return (
    <section
      className="provider-model-picker"
      aria-labelledby={`${id}-heading`}
      aria-busy={state === "loading"}
    >
      <div className="provider-catalog-heading">
        <div>
          <h4 id={`${id}-heading`}>Available models</h4>
          <p>
            Load models from your provider, then choose one. You can also enter
            a model ID below.
          </p>
        </div>
        <button
          type="button"
          className="button secondary"
          disabled={disabled || state === "loading"}
          onClick={() => void load()}
        >
          {state === "loading"
            ? "Loading models…"
            : state === "error"
              ? "Retry loading models"
              : state === "loaded"
                ? "Refresh models"
                : "Load models"}
        </button>
      </div>
      {state === "loading" ? (
        <div className="provider-catalog-loading" role="status">
          <IconLoader2 className="spin-icon" size={20} aria-hidden="true" />
          <div>
            <strong>Loading models from your provider…</strong>
            <p className="provider-catalog-note">
              If a Colossus window is waiting for your input, complete or cancel
              it to continue.
            </p>
          </div>
        </div>
      ) : null}
      {state === "error" ? (
        <p role="alert" className="page-error">
          {error} Check your API base URL and sign-in details, then try again.
          You can also enter a model ID below.
        </p>
      ) : null}
      {state === "loaded" && models.length === 0 ? (
        <p role="status">
          This provider returned no models. Enter a model ID below to continue.
        </p>
      ) : null}
      {models.length > 0 ? (
        <>
          <label className="provider-model-search">
            <span>Search models</span>
            <input
              type="search"
              value={query}
              disabled={disabled}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Model name or ID"
            />
          </label>
          <p className="provider-catalog-count" role="status">
            {filtered.length} matching{" "}
            {filtered.length === 1 ? "model" : "models"}
            {filtered.length > 50
              ? " · Showing the first 50; search to narrow the list"
              : ""}
          </p>
          <div className="provider-model-results">
            {filtered.slice(0, 50).map((entry) => (
              <button
                type="button"
                key={entry.id}
                disabled={disabled}
                className={`provider-model-card${model === entry.id ? " is-selected" : ""}`}
                aria-pressed={model === entry.id}
                onClick={() => onSelect(entry)}
              >
                <strong>{entry.display_name ?? entry.id}</strong>
                {entry.display_name && entry.display_name !== entry.id ? (
                  <span className="provider-model-id">{entry.id}</span>
                ) : null}
                {entry.description ? (
                  <span className="provider-model-description">
                    {entry.description}
                  </span>
                ) : null}
                <span className="provider-model-facts">
                  {entry.context_window_tokens
                    ? `${entry.context_window_tokens.toLocaleString()} context tokens`
                    : "Context size not reported"}{" "}
                  ·{" "}
                  {entry.max_output_tokens
                    ? `${entry.max_output_tokens.toLocaleString()} output tokens`
                    : "Output limit not reported"}
                </span>
                <span className="provider-model-facts">
                  Tools: {capability(entry.tool_calls)} · Images:{" "}
                  {capability(entry.image_inputs)} · Streaming:{" "}
                  {capability(entry.streaming)}
                </span>
                {entry.supported_reasoning_efforts?.length ? (
                  <span className="provider-model-facts">
                    Reasoning: {entry.supported_reasoning_efforts.join(", ")}
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        </>
      ) : null}
      {model && !selected ? (
        <p className="provider-catalog-note">
          This model’s details have not been loaded from your provider. Review
          its limits and supported features before saving.
        </p>
      ) : selected ? (
        <p className="provider-catalog-note">
          Model details are filled in where your provider reports them. Features
          the provider does not report start turned off. Review the limits and
          supported features before saving.
        </p>
      ) : null}
    </section>
  );
}

function capability(value: boolean | undefined): string {
  return value === undefined ? "not reported" : value ? "yes" : "no";
}
