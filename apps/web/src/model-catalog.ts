export const REASONING_EFFORTS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;
export type ReasoningEffort = (typeof REASONING_EFFORTS)[number];

export interface ConfiguredModel {
  profile: string;
  model: string;
  provider: string;
  provider_profile: string;
  reasoning_effort: ReasoningEffort | null;
  default_roles: string[];
}

export function reasoningEffort(value: string): ReasoningEffort | undefined {
  return REASONING_EFFORTS.find((effort) => effort === value);
}

/** Accept only the credential-free configured catalog, never posture labels. */
export function configuredModels(value: unknown): ConfiguredModel[] {
  if (!Array.isArray(value) || value.length > 256) return [];
  const seen = new Set<string>();
  for (const entry of value) {
    if (
      !entry ||
      typeof entry !== "object" ||
      typeof entry.profile !== "string" ||
      !entry.profile ||
      entry.profile.length > 128 ||
      seen.has(entry.profile) ||
      typeof entry.model !== "string" ||
      typeof entry.provider !== "string" ||
      typeof entry.provider_profile !== "string" ||
      (entry.reasoning_effort !== null &&
        (typeof entry.reasoning_effort !== "string" ||
          !reasoningEffort(entry.reasoning_effort))) ||
      !Array.isArray(entry.default_roles) ||
      !entry.default_roles.every((role: unknown) => typeof role === "string")
    )
      return [];
    seen.add(entry.profile);
  }
  return value.map((entry) => ({
    profile: entry.profile,
    model: entry.model,
    provider: entry.provider,
    provider_profile: entry.provider_profile,
    reasoning_effort: entry.reasoning_effort,
    default_roles: [...entry.default_roles],
  }));
}
