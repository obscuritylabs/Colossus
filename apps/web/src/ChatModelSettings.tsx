import { useState } from "react";
import {
  ModelSettingsEditor,
  type ModelSettingsDraft,
} from "@colossus/ui/components/ModelSettingsEditor";
import { ProviderIcon } from "@colossus/ui/components/ProviderIcon";
import { projectPath, request } from "./api";
import {
  validProfile,
  type ChatSettingsWrite,
  type ChatSettingsSnapshot,
} from "./chat-settings";

export function ChatModelSettings({
  id,
  draft,
  project,
  saved,
  onApply,
  onCancel,
  onBusyChange,
}: {
  id: string;
  draft: ChatSettingsWrite;
  project: string;
  saved: ChatSettingsSnapshot;
  onApply: (value: ChatSettingsWrite) => void;
  onCancel: () => void;
  onBusyChange?: ((busy: boolean) => void) | undefined;
}) {
  const original = draft.models.profiles[id];
  const [value, setValue] = useState<ModelSettingsDraft>(() => ({
    resourceId: id || null,
    label: draft.labels?.models[id] ?? id,
    profile: id,
    providerProfile:
      original?.providerProfile ??
      Object.keys(draft.providers.profiles)[0] ??
      "",
    model: original?.model ?? "",
    contextWindowTokens: original?.contextWindowTokens ?? 32768,
    maxOutputTokens: original?.maxOutputTokens ?? 4096,
    reasoningEffort: (original?.reasoningEffort ??
      null) as ModelSettingsDraft["reasoningEffort"],
    toolCalls: (original?.capabilities?.toolCalls ??
      "auto") as ModelSettingsDraft["toolCalls"],
    streaming: (original?.capabilities?.streaming ??
      "auto") as ModelSettingsDraft["streaming"],
    imageInputs: (original?.capabilities?.imageInputs ??
      "auto") as ModelSettingsDraft["imageInputs"],
    serverCompaction: (original?.capabilities?.serverCompaction ??
      "auto") as ModelSettingsDraft["toolCalls"],
    declared: original?.capabilities
      ?.declared as ModelSettingsDraft["declared"],
  }));
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const provider = draft.providers.profiles[value.providerProfile];
  const canLoad = Boolean(
    provider &&
    JSON.stringify(provider) ===
      JSON.stringify(saved.providers.profiles[value.providerProfile]) &&
    draft.provider_authentication[value.providerProfile] ===
      saved.provider_authentication[value.providerProfile],
  );
  function reset(next: Partial<ModelSettingsDraft>) {
    setValue({
      ...value,
      contextWindowTokens: 32768,
      maxOutputTokens: 4096,
      reasoningEffort: null,
      declared: undefined,
      ...next,
    });
  }
  function apply() {
    if (
      !validProfile(value.profile) ||
      !value.label.trim() ||
      !value.model.trim()
    ) {
      setError(
        "Enter a display label, valid profile ID, and model identifier.",
      );
      return;
    }
    if (
      value.profile !== id &&
      Object.hasOwn(draft.models.profiles, value.profile)
    ) {
      setError("That profile ID already exists.");
      return;
    }
    if (value.maxOutputTokens > value.contextWindowTokens) {
      setError("Output tokens must fit the context window.");
      return;
    }
    const next = structuredClone(draft);
    if (id && id !== value.profile) delete next.models.profiles[id];
    next.models.profiles[value.profile] = {
      ...original,
      providerProfile: value.providerProfile,
      model: value.model.trim(),
      contextWindowTokens: value.contextWindowTokens,
      maxOutputTokens: value.maxOutputTokens,
      reasoningEffort: value.reasoningEffort,
      capabilities: {
        ...original?.capabilities,
        toolCalls: value.toolCalls,
        streaming: value.streaming,
        imageInputs: value.imageInputs,
        serverCompaction: value.serverCompaction ?? "auto",
        declared: value.declared,
      },
    };
    next.labels ??= { providers: {}, models: {} };
    if (id && id !== value.profile) {
      delete next.labels.models[id];
      for (const role of Object.keys(next.models.roles))
        if (next.models.roles[role] === id)
          next.models.roles[role] = value.profile;
    }
    next.labels.models[value.profile] = value.label.trim();
    if (!next.models.roles.primary) next.models.roles.primary = value.profile;
    onApply(next);
  }
  return (
    <>
      {error ? <p role="alert">{error}</p> : null}
      <ModelSettingsEditor
        draft={value}
        providers={Object.keys(draft.providers.profiles).map((profile) => ({
          profile,
          label: draft.labels?.providers[profile] ?? profile,
        }))}
        busy={pending}
        onChange={setValue}
        onCancel={onCancel}
        onSave={apply}
        connectionKey={JSON.stringify([
          project,
          saved.revision,
          value.providerProfile,
          provider,
        ])}
        catalogDisabled={!canLoad}
        catalogNotice={
          !canLoad ? (
            <p>
              Save the provider configuration and credential before loading
              models.
            </p>
          ) : null
        }
        providerIcon={(profile) => (
          <ProviderIcon
            provider={
              draft.providers.profiles[profile]
                ? {
                    kind: draft.providers.profiles[profile]!.kind,
                    baseUrl: draft.providers.profiles[profile]!.baseUrl ?? null,
                  }
                : null
            }
          />
        )}
        onLoadModels={async () => {
          setPending(true);
          onBusyChange?.(true);
          try {
            return (
              await request<{
                models: import("@colossus/ui/lib/provider-catalog").ProviderCatalogModel[];
              }>(
                projectPath(project) +
                  "/chat/settings/providers/" +
                  encodeURIComponent(value.providerProfile) +
                  "/models",
                { revision: saved.revision },
              )
            ).models;
          } finally {
            setPending(false);
            onBusyChange?.(false);
          }
        }}
        onProviderChange={(profile) =>
          reset({ providerProfile: profile, model: "" })
        }
        onModelIdentifierChange={(model) => reset({ model })}
        onSelectCatalogModel={(entry) => {
          const context = entry.context_window_tokens ?? 32768;
          setValue({
            ...value,
            model: entry.id,
            contextWindowTokens: context,
            maxOutputTokens: Math.min(
              entry.max_output_tokens ?? 4096,
              Math.floor(context / 2),
            ),
            reasoningEffort: null,
            declared: {
              toolCalls: entry.tool_calls ?? null,
              streaming: entry.streaming ?? null,
              imageInputs: entry.image_inputs ?? null,
              serverCompaction: entry.server_compaction ?? null,
            },
            label: value.label || entry.display_name || entry.id,
            profile:
              value.profile ||
              entry.id.replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 64),
          });
        }}
      />
    </>
  );
}
