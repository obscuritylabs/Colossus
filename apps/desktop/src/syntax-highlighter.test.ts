import { describe, expect, it } from "vitest";

import { highlightSource } from "./syntax-highlighter";

describe("syntax highlighter", () => {
  it("uses distinct token palettes for light and dark previews", async () => {
    const source = "const ready: boolean = true;";
    const [dark, light] = await Promise.all([
      highlightSource(source, "typescript", "dark"),
      highlightSource(source, "typescript", "light"),
    ]);

    expect(dark.map((line) => line.map((token) => token.content))).toEqual(
      light.map((line) => line.map((token) => token.content)),
    );
    expect(dark.flat().map((token) => token.color)).not.toEqual(
      light.flat().map((token) => token.color),
    );
  });
});
