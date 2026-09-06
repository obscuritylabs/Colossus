/** Development-only interaction fixture. Native behavior is verified separately. */
import { useState } from "react";
import { PluginsSurface } from "../components/PluginsSurface";
import { PluginConfigurationEditor } from "../components/PluginConfigurationEditor";

export default function PluginStudio() {
  const [target, setTarget] = useState("local");
  const [skills, setSkills] = useState<string[]>([]);
  const [settings, setSettings] = useState<Record<string, unknown>>({});
  return (
    <main>
      <nav aria-label="Test target">
        <button
          onClick={() => {
            setTarget("local");
            setSkills([]);
          }}
        >
          Managed Local
        </button>
        <button
          onClick={() => {
            setTarget("external");
            setSkills([]);
          }}
        >
          External
        </button>
        <button
          onClick={() => {
            setTarget("old");
            setSkills([]);
          }}
        >
          Unsupported target
        </button>
      </nav>
      <section aria-label="Conversation skills">
        {skills.map((id) => (
          <button
            key={id}
            onClick={() =>
              setSkills((current) => current.filter((skill) => skill !== id))
            }
          >
            Remove {id}
          </button>
        ))}
        <button onClick={() => setSkills([])}>New conversation</button>
      </section>
      <PluginsSurface
        key={target}
        targetId={target}
        supported={target !== "old"}
        selections={skills}
        onUseSkill={(id) =>
          setSkills((current) => [...new Set([...current, id])])
        }
      />
      <section aria-label="Plugin settings">
        {[
          "plugins.trustProfiles",
          "plugins.registries",
          "plugins.mcpServers",
        ].map((id) => (
          <PluginConfigurationEditor
            key={id}
            id={id}
            value={settings[id]}
            onChange={(value) =>
              setSettings((current) => ({ ...current, [id]: value }))
            }
          />
        ))}
      </section>
      <output aria-label="Test settings value">
        {JSON.stringify(settings)}
      </output>
    </main>
  );
}
