import { IconArrowUpRight } from "@tabler/icons-react";
import {
  composerModelIdentity,
  type ComposerModelContext,
  type ComposerModelSelection,
} from "../composer-model";
import type { ReasoningEffort } from "../types";
import { ModelPicker } from "@colossus/ui/model-picker";
import { ProviderIcon } from "./ProviderIcon";
import "./composer-model.css";

export function ComposerModelChip({
  context,
  role,
  onOpenSettings,
  selection,
  onSelectionChange,
  disabled = false,
}: {
  context?: ComposerModelContext | undefined;
  role: string;
  onOpenSettings?: (() => void) | undefined;
  selection?: ComposerModelSelection | undefined;
  onSelectionChange?: ((selection: ComposerModelSelection) => void) | undefined;
  disabled?: boolean;
}) {
  const identity = composerModelIdentity(context, role, selection);
  const canOpen = context?.targetKind === "managed_local" && onOpenSettings;
  const title = `${identity.model} · ${identity.providerLabel}${identity.reasoningEffort ? ` · Reasoning: ${identity.reasoningEffort}` : ""}`;
  if (context?.targetKind === "managed_local" && onSelectionChange) {
    const modelDefault = composerModelIdentity(
      context,
      role,
      selection?.modelProfile ? { modelProfile: selection.modelProfile } : {},
    );
    const efforts: ReasoningEffort[] = [
      "none",
      "minimal",
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
      "ultra",
    ];
    return (
      <div className="composer-model-controls">
        <ModelPicker
          disabled={disabled}
          model={selection?.modelProfile ?? ""}
          label={identity.model}
          detail={identity.providerLabel}
          icon={<ProviderIcon provider={identity.provider} size={16} />}
          models={[
            {
              value: "",
              label: "Role default",
              detail: `${composerModelIdentity(context, role).model} · ${composerModelIdentity(context, role).providerLabel}`,
              icon: (
                <ProviderIcon
                  provider={composerModelIdentity(context, role).provider}
                  size={16}
                />
              ),
            },
            ...context.configuration.models.map((model) => {
              const option = composerModelIdentity(context, role, {
                modelProfile: model.profile,
              });
              return {
                value: model.profile,
                label: model.model,
                detail: `${option.providerLabel} · ${model.profile} (${model.providerProfile})`,
                icon: <ProviderIcon provider={option.provider} size={16} />,
              };
            }),
          ]}
          onModelChange={(profile) => {
            if (profile === (selection?.modelProfile ?? "")) return;
            onSelectionChange(profile ? { modelProfile: profile } : {});
          }}
          reasoning={selection?.reasoningEffort ?? ""}
          reasoningLabel={
            selection?.reasoningEffort ??
            `Model default${modelDefault.reasoningEffort ? ` · ${modelDefault.reasoningEffort}` : ""}`
          }
          reasoningOptions={[
            { value: "", label: "Model default" },
            ...efforts.map((effort) => ({ value: effort, label: effort })),
          ]}
          onReasoningChange={(effort) => {
            const next = { ...selection };
            if (effort) next.reasoningEffort = effort as ReasoningEffort;
            else delete next.reasoningEffort;
            onSelectionChange(next);
          }}
          onOpenSettings={canOpen ? onOpenSettings : undefined}
        />
      </div>
    );
  }
  const content = (
    <>
      <ProviderIcon provider={identity.provider} size={16} />
      <span className="composer-model-copy">
        <strong>{identity.model}</strong>
        <span>{identity.providerLabel}</span>
      </span>
      {identity.reasoningEffort ? (
        <span
          className="composer-reasoning"
          title={`Reasoning effort: ${identity.reasoningEffort}`}
        >
          {identity.reasoningEffort} reasoning
        </span>
      ) : null}
      {canOpen ? (
        <IconArrowUpRight
          className="composer-model-settings-icon"
          size={14}
          aria-hidden="true"
        />
      ) : null}
    </>
  );
  return canOpen ? (
    <button
      type="button"
      className="composer-model-chip"
      disabled={disabled}
      title={`Open model settings: ${title}`}
      aria-label={`Model settings: ${title}`}
      onClick={onOpenSettings}
    >
      {content}
    </button>
  ) : (
    <span className="composer-model-chip" title={title}>
      {content}
    </span>
  );
}
