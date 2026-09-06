import { describe, expect, it } from "vitest";
import { pluginIconSource, pluginMentionSuggestions } from "./plugins";

const skills = ["coding", "plugin-authoring"].map((name) => ({
  id: `colossus/${name}`,
  plugin: "colossus",
  name,
  description: name,
  compatibility: null,
  allowed_tools: null,
}));
describe("leading plugin mention completion", () => {
  it("offers qualified skills and preserves preceding known selections", () => {
    expect(pluginMentionSuggestions("@colossus/pl", skills)[0]?.command).toBe(
      "@colossus/plugin-authoring ",
    );
    expect(
      pluginMentionSuggestions(" @colossus/coding @", skills).map(
        (item) => item.command,
      ),
    ).toEqual([" @colossus/coding @colossus/"]);
  });
  it("leaves ordinary text, unknown mentions and completed mentions intact", () => {
    for (const prompt of [
      "Email @colossus/pl",
      "@unknown @",
      "@colossus/coding ",
      "@someone",
    ])
      expect(pluginMentionSuggestions(prompt, skills)).toEqual([]);
  });
});

describe("plugin identity", () => {
  it("offers each plugin once before drilling into its remaining skills", () => {
    const icon = "data:image/png;base64,iVBORw0KGgo=";
    const catalog = skills.map((skill) => ({
      ...skill,
      icon_data_url: icon,
      plugin_description: "Colossus development skills",
    }));
    expect(pluginMentionSuggestions("@", catalog)).toEqual([
      {
        command: "@colossus/",
        label: "colossus",
        plugin: "colossus",
        icon,
        description: "Colossus development skills",
        group: "Plugin",
      },
    ]);
    expect(pluginMentionSuggestions("@colossus/", catalog)).toHaveLength(2);
    expect(
      pluginMentionSuggestions(
        "@colossus/coding @colossus/plugin-authoring @",
        catalog,
      ),
    ).toEqual([]);
  });
  it("admits only bounded PNG display data and rejects remote or executable sources", () => {
    const png = "data:image/png;base64,iVBORw0KGgo=";
    expect(pluginIconSource(png)).toBe(png);
    for (const invalid of [
      null,
      undefined,
      "",
      "https://example.test/track.png",
      "file:///tmp/icon.png",
      "data:image/svg+xml,<svg/>",
      "data:image/png;base64,PHN2Zz4=",
      png + "a".repeat(90_000),
    ]) {
      expect(pluginIconSource(invalid)).toBeUndefined();
    }
  });
});
