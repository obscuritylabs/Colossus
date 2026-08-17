import { describe, expect, it } from "vitest";

import {
  MAX_ASIDE_PANE_WIDTH,
  MIN_ASIDE_PANE_WIDTH,
  clampAsidePaneWidth,
  defaultAsidePaneWidth,
} from "./aside-pane-width";

describe("Aside pane width", () => {
  it("uses the existing split ratio for a new pane", () => {
    expect(defaultAsidePaneWidth(1_200)).toBe(636);
  });

  it("preserves a usable thread while constraining the Aside", () => {
    expect(clampAsidePaneWidth(50, 1_200)).toBe(MIN_ASIDE_PANE_WIDTH);
    expect(clampAsidePaneWidth(2_000, 1_200)).toBe(MAX_ASIDE_PANE_WIDTH);
    expect(clampAsidePaneWidth(600, 700)).toBe(302);
  });
});
