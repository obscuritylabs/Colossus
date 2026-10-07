import { MODEL_ROLES, type RoleModel } from "./ModelRoleRouting";

export function modelRouteGroups(
  roles: Record<string, string>,
  models: RoleModel[],
) {
  const available = new Map(models.map((model) => [model.profile, model]));
  const groups = new Map<
    string,
    {
      profile: string;
      model: RoleModel | undefined;
      roles: { id: string; label: string; inherited: boolean }[];
    }
  >();
  for (const role of MODEL_ROLES) {
    const assigned = roles[role.id];
    const inherited = role.id !== "primary" && !assigned;
    const profile = assigned || (inherited ? roles.primary : "") || "";
    let group = groups.get(profile);
    if (!group) {
      group = { profile, model: available.get(profile), roles: [] };
      groups.set(profile, group);
    }
    group.roles.push({ id: role.id, label: role.label, inherited });
  }
  return [...groups.values()];
}
