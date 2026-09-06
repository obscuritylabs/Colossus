/** Credential-free runtime inventory. Field names intentionally match the shared contract. */
export interface PluginSkill {
  /** Display-only metadata attached from the owning inventory entry. */
  icon_data_url?: string | null;
  plugin_description?: string | null;
  id: string;
  plugin: string;
  name: string;
  description: string;
  compatibility: string | null;
  allowed_tools: string | null;
}
export interface PluginEntry {
  icon_data_url?: string | null;
  manifest: {
    name: string;
    version: string | null;
    description: string | null;
  };
  digest: string;
  source: string;
  origin: "bundled" | "installed";
  status: "enabled" | "disabled" | "uninstalled";
  available: boolean;
  unavailable_reason: string | null;
  actions: string[];
  trust: {
    trusted: boolean;
    profile: string | null;
    signer: string | null;
    method: string;
  };
  skills: PluginSkill[];
  mcp_servers: {
    id: string;
    name: string;
    transport: string;
    enabled: boolean;
    status: string;
  }[];
  diagnostics: {
    kind: string;
    name: string | null;
    code: string;
    detail: string;
  }[];
}
export interface PluginInventory {
  plugins: PluginEntry[];
  managementAvailable: boolean;
}
export interface PluginResource {
  skill_id: string;
  path: string;
  size: number;
  text: boolean;
}
export type PluginSource =
  | { kind: "directory"; path: string }
  | { kind: "layout" | "archive"; path: string; digest: string | null }
  | { kind: "reference"; registry: string; reference: string };
export type PluginRequest =
  | { operation: "validate"; path: string }
  | {
      operation: "verify";
      path: string;
      digest: string | null;
      trust_profile: string;
    }
  | { operation: "verify_installed"; name: string; digest: string }
  | { operation: "install"; source: PluginSource; trust_profile: string }
  | {
      operation: "enable";
      name: string;
      digest: string;
      allow_untrusted: boolean;
    }
  | { operation: "disable"; name: string }
  | { operation: "update"; name: string; registry: string; reference: string }
  | {
      operation: "uninstall";
      name: string;
      digest: string;
      purge_data: boolean;
    }
  | { operation: "gc" }
  | { operation: "package"; directory: string; output: string }
  | { operation: "pull"; registry: string; reference: string; output: string }
  | { operation: "push"; registry: string; reference: string; layout: string }
  | { operation: "export"; name: string; output: string };

export function filterPlugins(
  plugins: readonly PluginEntry[],
  query: string,
): PluginEntry[] {
  const needle = query.trim().toLocaleLowerCase();
  return plugins.filter((plugin) =>
    [
      plugin.manifest.name,
      plugin.manifest.description ?? "",
      ...plugin.skills.map((skill) => `${skill.id} ${skill.description}`),
    ]
      .join(" ")
      .toLocaleLowerCase()
      .includes(needle),
  );
}

export function pluginSelectionKey(
  target: string | null,
  session: string | undefined,
): string {
  return JSON.stringify([target, session ?? "draft"]);
}

/** Complete only a leading mention sequence; unknown mentions remain ordinary text. */
export function pluginMentionSuggestions(
  prompt: string,
  skills: readonly PluginSkill[],
) {
  const match = /^(\s*(?:@[^\s]+\s+)*)@([^\s]*)$/.exec(prompt);
  if (!match) return [];
  const prefix = match[1] ?? "";
  const known = new Set(skills.map((skill) => skill.id));
  const selected = prefix
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((token) => token.slice(1));
  if (selected.some((id) => !known.has(id))) return [];
  const query = match[2] ?? "";
  const remaining = skills.filter((skill) => !selected.includes(skill.id));
  if (!query.includes("/")) {
    const groups = new Map<string, PluginSkill[]>();
    for (const skill of remaining) {
      const group = groups.get(skill.plugin) ?? [];
      group.push(skill);
      groups.set(skill.plugin, group);
    }
    return [...groups.entries()]
      .filter(([name]) => name.startsWith(query))
      .sort(([left], [right]) => left.localeCompare(right))
      .slice(0, 12)
      .map(([name, available]) => ({
        command: `${prefix}@${name}/`,
        label: name,
        plugin: name,
        icon: available[0]?.icon_data_url,
        description:
          available[0]?.plugin_description ??
          `${available.length} ${available.length === 1 ? "skill" : "skills"} available`,
        group: "Plugin",
      }));
  }
  return skills
    .filter(
      (skill) => skill.id.startsWith(query) && !selected.includes(skill.id),
    )
    .sort((left, right) => left.id.localeCompare(right.id))
    .slice(0, 12)
    .map((skill) => ({
      command: `${prefix}@${skill.id} `,
      label: skill.name,
      plugin: skill.plugin,
      icon: skill.icon_data_url,
      description: skill.description,
      group: "Skill",
    }));
}

/** Only bounded, embedded PNGs may reach an img src, including from external runtimes. */
export function pluginIconSource(
  value: string | null | undefined,
): string | undefined {
  if (
    !value ||
    value.length > 87_406 ||
    !/^data:image\/png;base64,iVBORw0KGgo[A-Za-z0-9+/]*={0,2}$/.test(value)
  )
    return undefined;
  return value;
}
