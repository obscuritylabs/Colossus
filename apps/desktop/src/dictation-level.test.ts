import { describe, expect, it } from "vitest";
import { DictationLevelMeter } from "./dictation-level";

describe("dictation input display", () => {
  it("settles steady room noise into silence and responds to speech above it", () => {
    const meter = new DictationLevelMeter();
    for (let index = 0; index < 24; index += 1) {
      const value = meter.update(80 + (index % 5));
      if (index >= 7) expect(value).toBe(0);
    }
    expect(meter.update(180)).toBeGreaterThan(0.5);
    expect(meter.update(82)).toBe(0);
    expect(meter.update(0)).toBe(0);
  });

  it("shows speech immediately and does not calibrate away sustained strong input", () => {
    const meter = new DictationLevelMeter();
    expect(meter.update(100)).toBeGreaterThan(0);
    for (let index = 0; index < 80; index += 1) {
      expect(meter.update(200)).toBeGreaterThan(0.5);
    }
    expect(meter.update(255)).toBe(1);
  });

  it("relearns quieter and louder backgrounds using a bounded recent window", () => {
    const meter = new DictationLevelMeter();
    for (let index = 0; index < 24; index += 1) meter.update(100);
    expect(meter.update(100)).toBe(0);
    for (let index = 0; index < 24; index += 1) meter.update(30);
    expect(meter.update(70)).toBeGreaterThan(0);
    for (let index = 0; index < 24; index += 1) meter.update(110);
    expect(meter.update(110)).toBe(0);
    expect(meter.update(170)).toBeGreaterThan(0);
  });
});
