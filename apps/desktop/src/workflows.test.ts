import { describe, expect, it } from "vitest";
import { canonicalJson, firstOccurrence, scheduleInputs } from "./workflows";

describe("reviewed schedule intent", () => {
  it("freezes UTC and rejects invalid dates and non-object inputs", () => {
    expect(firstOccurrence("2026-10-05T09:30", "utc")).toBe(
      "2026-10-05T09:30:00.000Z",
    );
    expect(() => firstOccurrence("2026-02-30T09:30", "utc")).toThrow(
      "does not exist",
    );
    for (const input of [
      "null",
      "[]",
      '"text"',
      "invalid",
      JSON.stringify({ x: "x".repeat(65536) }),
    ])
      expect(() => scheduleInputs(input)).toThrow();
  });
  it("reconciles equivalent nested JSON independently of insertion order", () => {
    expect(canonicalJson({ z: [1, { b: false, a: null }], a: "x" })).toBe(
      canonicalJson({ a: "x", z: [1, { a: null, b: false }] }),
    );
    expect(canonicalJson({ a: [1, 2] })).not.toBe(canonicalJson({ a: [2, 1] }));
  });
});
