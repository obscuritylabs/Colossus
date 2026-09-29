import { useEffect, useRef, useState } from "react";
import { getProviderPresets, listSetupPackages } from "../api";
import { SETUP_CHANGED_EVENT } from "../setupPackages";
import { MarkdownContent } from "./MarkdownContent";
import { BrowserLinkContext } from "./browser/BrowserLink";
import { openSetupLink } from "../api";
import {
  matchingProviderPreset,
  type ProviderPreset,
} from "../providerCatalog";
import type { ProviderKind } from "../types";
import { DropdownSelect } from "./DropdownSelect";
import { ProviderIcon } from "./ProviderIcon";

export function ProviderPresetSelect({
  kind,
  baseUrl,
  busy,
  onSelect,
  defaultPresetId,
}: {
  kind: ProviderKind;
  baseUrl: string;
  busy: boolean;
  onSelect: (preset: ProviderPreset) => void;
  defaultPresetId?: string;
}) {
  const [presets, setPresets] = useState<ProviderPreset[]>([]);
  const [error, setError] = useState(false);
  const [retry, setRetry] = useState(0);
  const selectRef = useRef(onSelect);
  selectRef.current = onSelect;
  const initialConnection = useRef({ kind, baseUrl });
  const allowDefault = useRef(true);
  if (
    initialConnection.current.kind !== kind ||
    initialConnection.current.baseUrl !== baseUrl
  )
    allowDefault.current = false;
  useEffect(() => {
    let active = true;
    setError(false);
    void Promise.all([
      getProviderPresets(),
      listSetupPackages().catch(() => []),
    ])
      .then(([builtins, packages]): ProviderPreset[] => [
        ...packages.flatMap((item) =>
          item.providers.map((provider): ProviderPreset => ({
            id: `setup:${item.id}:${provider.profile}`,
            label: provider.displayName,
            protocol:
              provider.kind === "open_ai_codex"
                ? "codex"
                : provider.kind === "openai_responses"
                  ? "responses"
                  : "chat_completions",
            baseUrl: provider.baseUrl,
            credentialEnv: provider.credentialRequired ? "SETUP_API_KEY" : null,
            setup: { packageId: item.id, provider },
          })),
        ),
        ...builtins,
      ])
      .then(
        (result) => {
          if (!active) return;
          setPresets(result);
          const initial = result.find(
            (preset) => preset.id === defaultPresetId,
          );
          if (
            initial &&
            allowDefault.current &&
            initialConnection.current.baseUrl === ""
          ) {
            allowDefault.current = false;
            selectRef.current(initial);
          }
        },
        () => {
          if (active) setError(true);
        },
      );
    return () => {
      active = false;
    };
  }, [retry]);
  useEffect(() => {
    const refresh = () => setRetry((value) => value + 1);
    window.addEventListener(SETUP_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(SETUP_CHANGED_EVENT, refresh);
  }, []);
  const [linkError, setLinkError] = useState("");
  const selected = matchingProviderPreset(presets, kind, baseUrl);
  return (
    <div className="provider-preset-control">
      <label>
        <span>Provider</span>
        <DropdownSelect
          aria-label="Provider"
          value={selected?.id ?? ""}
          disabled={busy || presets.length === 0}
          renderOptionIcon={(option) =>
            option.value ? (
              <ProviderIcon
                presetId={option.value}
                customIcon={
                  presets.find((p) => p.id === option.value)?.setup?.provider
                    .icon ?? null
                }
                customDarkIcon={
                  presets.find((p) => p.id === option.value)?.setup?.provider
                    .darkIcon ?? null
                }
              />
            ) : null
          }
          onChange={(event) => {
            if (event.target.value === selected?.id) return;
            const preset = presets.find(
              (candidate) => candidate.id === event.target.value,
            );
            if (preset) {
              allowDefault.current = false;
              onSelect(preset);
            }
          }}
        >
          <option value="">
            {error
              ? "Providers unavailable"
              : presets.length === 0
                ? "Loading providers…"
                : "Choose a provider"}
          </option>
          {presets.map((preset) => (
            <option key={preset.id} value={preset.id}>
              {preset.label}
            </option>
          ))}
        </DropdownSelect>
      </label>
      {selected?.setup ? (
        <BrowserLinkContext.Provider
          value={(url) => {
            void openSetupLink(selected.setup!.packageId, url).catch(
              (error: unknown) =>
                setLinkError(
                  error instanceof Error
                    ? error.message
                    : "Could not open link.",
                ),
            );
          }}
        >
          <MarkdownContent
            content={selected.setup.provider.descriptionMarkdown}
          />
        </BrowserLinkContext.Provider>
      ) : null}
      {linkError ? <p role="alert">{linkError}</p> : null}
      {error ? (
        <p role="status">
          Could not load the provider list. You can still choose an API format
          and enter your provider’s base URL below.{" "}
          <button
            type="button"
            className="text-button"
            onClick={() => setRetry((value) => value + 1)}
          >
            Retry loading providers
          </button>
        </p>
      ) : null}
    </div>
  );
}
