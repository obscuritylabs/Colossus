import { describe, expect, it } from "vitest";

import { patchXtermForFrozenPrototype } from "./xterm-frozen-prototype";

describe("xterm frozen-prototype compatibility", () => {
  it("uses a null-prototype namespace for xterm's KeyCode helpers", () => {
    expect(patchXtermForFrozenPrototype("before})(Qn||={});after")).toBe(
      "before})(Qn||=Object.create(null));after",
    );
  });

  it("fails closed when the pinned bundle shape changes", () => {
    expect(() => patchXtermForFrozenPrototype("different bundle")).toThrow(
      "expected KeyCode namespace shape",
    );
    expect(() =>
      patchXtermForFrozenPrototype("})(Qn||={});})(Qn||={});"),
    ).toThrow("expected KeyCode namespace shape");
  });
});
