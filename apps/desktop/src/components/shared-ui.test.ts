import { describe, expect, it } from "vitest";
import { isComposerSendKey } from "@colossus/ui";

describe("shared composer keyboard behavior", () => {
  const enter = {
    key: "Enter",
    shiftKey: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    isComposing: false,
  };
  it("keeps multiline and IME input out of submission in both shortcut modes", () => {
    for (const shortcut of ["enter", "modEnter"] as const) {
      expect(
        isComposerSendKey(
          { ...enter, shiftKey: true, ctrlKey: true },
          shortcut,
        ),
      ).toBe(false);
      expect(
        isComposerSendKey({ ...enter, altKey: true, metaKey: true }, shortcut),
      ).toBe(false);
      expect(
        isComposerSendKey(
          { ...enter, isComposing: true, ctrlKey: true },
          shortcut,
        ),
      ).toBe(false);
    }
  });
  it("uses the host's configured send shortcut", () => {
    expect(isComposerSendKey(enter, "enter")).toBe(true);
    expect(isComposerSendKey(enter, "modEnter")).toBe(false);
    expect(isComposerSendKey({ ...enter, ctrlKey: true }, "modEnter")).toBe(
      true,
    );
    expect(isComposerSendKey({ ...enter, metaKey: true }, "modEnter")).toBe(
      true,
    );
  });
});
