import { UserError } from "./errors.js";
import {
  isModelSelection,
  isReasoningEffort,
  supportsModelSelection,
  type CapabilityView,
  type ModelProfileView,
  type ModelSelection,
} from "./model.js";

/** Release bounded authenticated metadata; never read worker configuration in the renderer. */
export function modelProfiles(
  profiles: readonly (Omit<ModelProfileView, "reasoningEffort"> & {
    reasoningEffort?: string | undefined;
  })[],
): ModelProfileView[] {
  const seen = new Set<string>();
  return profiles
    .slice(0, 256)
    .filter((profile) => {
      if (
        !isModelSelection({ modelProfile: profile.profile }) ||
        seen.has(profile.profile)
      )
        return false;
      seen.add(profile.profile);
      return true;
    })
    .map((profile) => ({
      profile: profile.profile,
      model: profile.model.slice(0, 256),
      provider: profile.provider.slice(0, 128),
      providerProfile: profile.providerProfile.slice(0, 128),
      defaultRoles: profile.defaultRoles.slice(0, 64),
      ...(isReasoningEffort(profile.reasoningEffort)
        ? { reasoningEffort: profile.reasoningEffort }
        : {}),
    }));
}

export function validateModelSelection(
  selection: ModelSelection,
  capabilities: readonly CapabilityView[],
  profiles: readonly ModelProfileView[],
  role: string,
): ModelSelection {
  if (!isModelSelection(selection))
    throw new UserError(
      "Choose a configured model and supported reasoning level.",
    );
  if (!Object.keys(selection).length) return {};
  if (!supportsModelSelection(capabilities))
    throw new UserError(
      "Model selection is unavailable for this worker connection.",
    );
  const profile = selection.modelProfile
    ? profiles.find((profile) => profile.profile === selection.modelProfile)
    : profiles.find((profile) => profile.defaultRoles.includes(role));
  if (selection.modelProfile && !profile)
    throw new UserError("Choose a model configured on the connected worker.");
  if (selection.reasoningEffort && profile?.provider === "echo")
    throw new UserError("Echo does not support reasoning effort.");
  return { ...selection };
}
