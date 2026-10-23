import { useEffect, useRef, useState } from "react";
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
  catalogEnabled = true,
  saved,
  onApply,
  onCancel,
  onBusyChange,
  busy = false,
  onDirtyChange,
}: {
  busy?: boolean;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  id: string;
  draft: ChatSettingsWrite;
  project: string;
  catalogEnabled?: boolean;
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
  const formValue = JSON.stringify(value);
  const openingValue = useRef(formValue);
  useEffect(() => {
    onDirtyChange?.(formValue !== openingValue.current);
  }, [formValue, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  const [pending, setPending] = useState(false);
  const provider = draft.providers.profiles[value.providerProfile];
  const networkAvailability =
    saved.connection_availability?.providers[value.providerProfile];
  const primaryAvailability = saved.connection_availability?.roles.primary;
  const catalogBlocked =
    networkAvailability?.enabled === false ||
    primaryAvailability?.enabled === false;
  const canLoad = Boolean(
    catalogEnabled &&
    !catalogBlocked &&
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
    if (busy || pending) return;
    if (!validProfile(value.profile)) {
      setError(
        "Use a nonreserved Profile ID of 1–128 letters, digits, hyphens, or underscores, starting with a letter or digit. Dots and slashes are allowed in Model identifier.",
      );
      return;
    }
    const label = value.label.trim();
    const model = value.model.trim();
    if (
      !label ||
      new TextEncoder().encode(label).length > 96 ||
      /[\u0000-\u001f\u007f-\u009f]/u.test(label)
    ) {
      setError(
        "Display label must contain 1–96 UTF-8 bytes and no control characters.",
      );
      return;
    }
    if (
      !model ||
      new TextEncoder().encode(model).length > 1024 ||
      /[\u0000-\u001f\u007f-\u009f]/u.test(model)
    ) {
      setError(
        "Model identifier must contain 1–1,024 UTF-8 bytes and no control characters. Dots and slashes are allowed.",
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
    if (!Object.hasOwn(draft.providers.profiles, value.providerProfile)) {
      setError("Choose an available provider before saving this model.");
      return;
    }
    if (
      ![value.maxOutputTokens, value.contextWindowTokens].every(
        (limit) => Number.isSafeInteger(limit) && limit > 0,
      )
    ) {
      setError("Token limits must be positive whole numbers.");
      return;
    }
    if (value.contextWindowTokens < 1024) {
      setError("Context window must be at least 1,024 tokens.");
      return;
    }
    if (value.maxOutputTokens >= value.contextWindowTokens) {
      setError("Maximum output must be smaller than the context window.");
      return;
    }
    const safetyMargin = Math.max(
      Math.ceil(value.contextWindowTokens / 10),
      512,
    );
    if (value.contextWindowTokens - value.maxOutputTokens <= safetyMargin) {
      setError(
        `Maximum output must be at most ${(value.contextWindowTokens - safetyMargin - 1).toLocaleString()} tokens to leave room for input and the ${safetyMargin.toLocaleString()} token safety reserve.`,
      );
      return;
    }
    const next = structuredClone(draft);
    if (id && id !== value.profile) delete next.models.profiles[id];
    next.models.profiles[value.profile] = {
      ...original,
      providerProfile: value.providerProfile,
      model,
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
    next.labels.models[value.profile] = label;
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
        busy={busy || pending}
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
              {catalogEnabled
                ? catalogBlocked
                  ? networkAvailability?.enabled === false
                    ? (networkAvailability.reason ??
                      "This provider is blocked by the project's Network access policy.")
                    : (primaryAvailability?.reason ??
                      "Choose an available primary model in Model routing before loading models.")
                  : "Save the provider configuration and credential before loading models."
                : "Configure model identifiers here. Live discovery uses the selected project's explicitly assigned credentials in Project Administration."}
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
              entry.id.replace(/[^a-zA-Z0-9_-]/g, "-").slice(0, 64),
          });
        }}
      />
    </>
  );
}
