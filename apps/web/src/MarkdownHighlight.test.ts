import { describe, expect, it } from "vitest";
// Exercise the source copied by the real shared-package installation.
import { highlightCode } from "../node_modules/@colossus/ui/src/markdown/highlight";

describe("bounded shared code highlighting", () => {
  it("uses semantic classes and preserves exact source for a language alias", async () => {
    const source = 'def greet():\n    return "hello"';
    const lines = await highlightCode(source, "py");
    expect(
      lines
        ?.map((line) => line.map((token) => token.content).join(""))
        .join("\n"),
    ).toBe(source);
    expect(
      lines?.flat().some((token) => token.className === "syntax-keyword"),
    ).toBe(true);
    expect(
      lines?.flat().every((token) => /^syntax-[a-z]+$/.test(token.className)),
    ).toBe(true);
  });
  it("keeps unsupported or oversized code as plain text", async () => {
    expect(await highlightCode("safe source", "unknown-language")).toBeNull();
    expect(await highlightCode("x".repeat(16_385), "python")).toBeNull();
  });
  it("bounds blank line fragments as well as colored tokens", async () => {
    expect(await highlightCode("\n".repeat(16_000), "python")).toBeNull();
  });
});
