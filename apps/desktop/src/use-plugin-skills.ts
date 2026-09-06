import { useEffect, useState } from "react";
import { getPluginInventory } from "./api";
import type { PluginSkill } from "./plugins";

/** Fetch credential-free completion metadata only while the composer needs it. */
export function usePluginSkills(
  targetId: string | null,
  supported: boolean,
  needed: boolean,
) {
  const [catalog, setCatalog] = useState<{
    target: string;
    skills: PluginSkill[];
  } | null>(null);
  useEffect(() => {
    if (!targetId || !supported || !needed) return;
    let current = true;
    let pending = false;
    const refresh = async () => {
      if (pending || document.visibilityState !== "visible") return;
      pending = true;
      try {
        const inventory = await getPluginInventory(targetId);
        if (current)
          setCatalog({
            target: targetId,
            skills: inventory.plugins
              .filter((plugin) => plugin.available)
              .flatMap((plugin) =>
                plugin.skills.map((skill) => ({
                  ...skill,
                  icon_data_url: plugin.icon_data_url ?? null,
                  plugin_description: plugin.manifest.description,
                })),
              ),
          });
      } catch {
        // Do not retain stale completion metadata after an authorization or transport failure.
        if (current) setCatalog(null);
      } finally {
        pending = false;
      }
    };
    void refresh();
    window.addEventListener("focus", refresh);
    const timer = window.setInterval(refresh, 15_000);
    return () => {
      current = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
    };
  }, [targetId, supported, needed]);
  return supported && catalog?.target === targetId ? catalog.skills : null;
}
