import { describe, expect, it } from "vitest";
import { pluginMentionSuggestions } from "./plugins";

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
    ).toEqual([" @colossus/coding @colossus/plugin-authoring "]);
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
