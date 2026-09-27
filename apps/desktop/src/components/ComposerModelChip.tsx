import { IconArrowUpRight } from "@tabler/icons-react";
import {
  composerModelIdentity,
  type ComposerModelContext,
} from "../composer-model";
import { ProviderIcon } from "./ProviderIcon";
import "./composer-model.css";

export function ComposerModelChip({
  context,
  role,
  onOpenSettings,
}: {
  context?: ComposerModelContext | undefined;
  role: string;
  onOpenSettings?: (() => void) | undefined;
}) {
  const identity = composerModelIdentity(context, role);
  const canOpen = context?.targetKind === "managed_local" && onOpenSettings;
  const title = `${identity.model} · ${identity.providerLabel}${identity.reasoningEffort ? ` · Reasoning: ${identity.reasoningEffort}` : ""}`;
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
