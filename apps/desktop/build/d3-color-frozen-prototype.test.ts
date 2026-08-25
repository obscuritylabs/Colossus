import { describe, expect, it } from "vitest";

import { patchD3ColorForFrozenPrototype } from "./d3-color-frozen-prototype";

const assignment = "  prototype.constructor = constructor;";

describe("d3-color frozen-prototype compatibility", () => {
  it("defines an own constructor instead of assigning through a frozen prototype", () => {
    const patched = patchD3ColorForFrozenPrototype(
      `before\n${assignment}\nafter`,
    );
    expect(patched).toContain(
      'Object.defineProperty(prototype, "constructor", {',
    );
    expect(patched).not.toContain(assignment);
  });

  it("fails closed when the pinned module shape changes", () => {
    expect(() => patchD3ColorForFrozenPrototype("different module")).toThrow(
      "expected constructor assignment shape",
    );
    expect(() =>
      patchD3ColorForFrozenPrototype(`${assignment}\n${assignment}`),
    ).toThrow("expected constructor assignment shape");
  });
});
