const WINDOW = 24;
const WARMUP = 8;
const MAX_FLOOR = 128; // About -30 dBFS in the native byte scale.
const MARGIN = 8; // About 2 dB above the estimated background.

/** A display-only noise gate; captured audio and recognition stay unchanged. */
export class DictationLevelMeter {
  private recent: number[] = [];

  update(level: number): number {
    this.recent.push(level);
    if (this.recent.length > WINDOW) this.recent.shift();
    const ordered = [...this.recent].sort((a, b) => a - b);
    const floor =
      ordered.length < WARMUP
        ? 0
        : Math.min(MAX_FLOOR, ordered[Math.floor(ordered.length / 5)]!);
    const threshold = floor + MARGIN;
    return Math.max(0, Math.min(1, (level - threshold) / (255 - threshold)));
  }
}
