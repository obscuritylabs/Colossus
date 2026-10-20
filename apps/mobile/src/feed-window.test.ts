import { describe, expect, it } from "vitest";
import {
  FeedWindow,
  MAX_FEED_EVENTS,
  MAX_FEED_BYTES,
  streamCursor,
} from "./feed-window";
import type { Update } from "@colossus/control-plane-client";
const event = (sequence: number, output = "released"): Update => ({
  run_id: "run-1",
  task_id: "task-1",
  sequence,
  created_at: "2026-10-16T00:00:00Z",
  update: { output_delta: output },
});
describe("bounded live activity", () => {
  it("deduplicates replay but refuses conflicting identities", () => {
    const window = new FeedWindow();
    expect(window.append(event(1))).toBe(true);
    expect(window.append(event(1))).toBe(false);
    expect(() => window.append(event(1, "different"))).toThrow("conflicting");
    expect(window.values()).toHaveLength(1);
  });
  it("keeps following new events after the retained window fills", () => {
    const window = new FeedWindow();
    for (let sequence = 1; sequence <= MAX_FEED_EVENTS + 20; sequence++)
      window.append(event(sequence));
    expect(window.limited).toBe(true);
    expect(window.values()).toHaveLength(MAX_FEED_EVENTS);
    expect(window.values()[0]?.sequence).toBe(21);
    expect(window.values().at(-1)?.sequence).toBe(MAX_FEED_EVENTS + 20);
  });
  it("never reintroduces evicted replay over the newer retained tail", () => {
    const window = new FeedWindow();
    for (let sequence = 1; sequence <= MAX_FEED_EVENTS + 1; sequence++)
      window.append(event(sequence));
    expect(window.append(event(1))).toBe(false);
    expect(window.values()[0]?.sequence).toBe(2);
    expect(window.values().at(-1)?.sequence).toBe(MAX_FEED_EVENTS + 1);
    expect(() => window.append(event(2, "changed"))).toThrow("conflicting");
  });
  it("bounds run identities before retaining replay metadata", () => {
    const window = new FeedWindow();
    expect(() =>
      window.append({ ...event(1), run_id: "x".repeat(257) }),
    ).toThrow("invalid");
    expect(() =>
      window.append({ ...event(1), task_id: "task\nprivate" }),
    ).toThrow("invalid");
  });
  it("preserves conversation cursor precision across the full unsigned range", () => {
    expect(streamCursor("00042")).toBe("42");
    expect(streamCursor("9007199254740993")).toBe("9007199254740993");
    expect(streamCursor("18446744073709551615")).toBe("18446744073709551615");
    for (const cursor of [
      "-1",
      "cursor-1",
      "18446744073709551616",
      "1&after=2",
    ])
      expect(() => streamCursor(cursor)).toThrow("invalid activity cursor");
  });
  it("bounds bytes while keeping later useful updates", () => {
    const window = new FeedWindow();
    window.append(event(1, "x".repeat(MAX_FEED_BYTES)));
    window.append(event(2));
    expect(window.limited).toBe(true);
    expect(window.values()).toEqual([event(2)]);
  });
});
