import { describe, expect, it } from "vitest";
import { historyWindow } from "./VirtualThreadList";

describe("historyWindow", () => {
  it("bounds rows for 5000 records, including a far older row", () => {
    const offsets = Array.from({ length: 5001 }, (_, i) => i * 45);
    const first = historyWindow(offsets, 0, 900);
    const last = historyWindow(offsets, 4990 * 45, 900);
    expect(first.start).toBe(0);
    expect(first.end - first.start).toBeLessThan(40);
    expect(last.start).toBeGreaterThan(4096);
    expect(last.end).toBe(5000);
    expect(last.end - last.start).toBeLessThan(40);
  });

  it("uses measured variable heights without clipping tall rows", () => {
    const window = historyWindow([0, 20, 1020, 1040, 2040], 700, 100);
    expect(window.start).toBe(1);
    expect(window.end).toBeGreaterThan(2);
    expect(historyWindow([0], 0, 900)).toEqual({ start: 0, end: 0 });
  });
});
