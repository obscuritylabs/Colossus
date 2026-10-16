import { ModelPicker } from "@colossus/ui/model-picker";
import {
  REASONING_EFFORTS,
  supportsModelSelection,
  type ModelSelection,
  type WorkView,
} from "../src/model.js";

/** The extension host owns discovery, validation, and conversation preferences. */
export function ComposerModelPicker({
  view,
  onChange,
}: {
  view: WorkView;
  onChange: (selection: ModelSelection) => void;
}) {
  if (
    !view.connected ||
    !supportsModelSelection(view.capabilities) ||
    !view.modelProfiles.length
  )
    return null;
  const selection = view.modelSelection;
  const roleDefault = view.modelProfiles.find(
    (model) => model.defaultRoles.length > 0,
  );
  const selected =
    view.modelProfiles.find(
      (model) => model.profile === selection.modelProfile,
    ) ?? roleDefault;
  const detail = (model: typeof selected) =>
    model
      ? `${model.provider} · ${model.profile} (${model.providerProfile})`
      : "Uses the worker's configured role route";
  return (
    <ModelPicker
      model={selection.modelProfile ?? ""}
      models={[
        { value: "", label: "Role default", detail: detail(roleDefault) },
        ...view.modelProfiles.map((model) => ({
          value: model.profile,
          label: model.model,
          detail: detail(model),
        })),
      ]}
      label={selected?.model ?? "Role default"}
      detail={detail(selected)}
      reasoning={selection.reasoningEffort ?? ""}
      reasoningLabel={
        selection.reasoningEffort ??
        `Model default${selected?.reasoningEffort ? ` · ${selected.reasoningEffort}` : ""}`
      }
      reasoningOptions={
        selected?.provider === "echo"
          ? []
          : [
              { value: "", label: "Model default" },
              ...REASONING_EFFORTS.map((effort) => ({
                value: effort,
                label: effort,
              })),
            ]
      }
      onModelChange={(profile) => {
        if (profile !== (selection.modelProfile ?? ""))
          onChange(profile ? { modelProfile: profile } : {});
      }}
      onReasoningChange={(effort) => {
        const next = { ...selection };
        if (effort)
          next.reasoningEffort = effort as (typeof REASONING_EFFORTS)[number];
        else delete next.reasoningEffort;
        onChange(next);
      }}
      disabled={view.busy && !view.sessionId}
    />
  );
}
