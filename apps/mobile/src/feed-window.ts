import type { Update } from "@colossus/control-plane-client";

export const MAX_FEED_EVENTS = 1000;
export const MAX_FEED_BYTES = 1024 * 1024;

/** Keep following new released activity when old events leave the bounded window. */
export class FeedWindow {
  private entries: {
    update: Update;
    size: number;
    json: string;
    key: string;
  }[] = [];
  private identities = new Map<string, string>();
  private bytes = 0;
  private evicted = new Map<string, number>();
  limited = false;

  append(update: Update): boolean {
    if (
      !validIdentity(update.run_id) ||
      (update.task_id !== undefined && !validIdentity(update.task_id)) ||
      !Number.isSafeInteger(update.sequence) ||
      update.sequence < 1 ||
      !update.update ||
      typeof update.update !== "object" ||
      Array.isArray(update.update)
    )
      throw new Error(
        "The runtime returned invalid activity. Reconnect to retry.",
      );
    const key = JSON.stringify([
      update.task_id ?? "",
      update.run_id,
      update.sequence,
    ]);
    const json = JSON.stringify(update);
    const previous = this.identities.get(key);
    if (previous !== undefined) {
      if (previous !== json)
        throw new Error(
          "The runtime returned conflicting activity. Reconnect to retry.",
        );
      return false;
    }
    const run = JSON.stringify([update.task_id ?? "", update.run_id]);
    if (update.sequence <= (this.evicted.get(run) ?? 0)) return false;
    const size = new TextEncoder().encode(json).byteLength;
    if (size > MAX_FEED_BYTES) {
      this.limited = true;
      return false;
    }
    this.entries.push({ update, size, json, key });
    this.identities.set(key, json);
    this.bytes += size;
    while (
      this.entries.length > MAX_FEED_EVENTS ||
      this.bytes > MAX_FEED_BYTES
    ) {
      const oldest = this.entries.shift()!;
      this.bytes -= oldest.size;
      this.identities.delete(oldest.key);
      const oldRun = JSON.stringify([
        oldest.update.task_id ?? "",
        oldest.update.run_id,
      ]);
      const through = Math.max(
        this.evicted.get(oldRun) ?? 0,
        oldest.update.sequence,
      );
      this.evicted.delete(oldRun);
      this.evicted.set(oldRun, through);
      if (this.evicted.size > MAX_FEED_EVENTS)
        this.evicted.delete(this.evicted.keys().next().value!);
      this.limited = true;
    }
    return true;
  }
  values(): Update[] {
    return this.entries.map((entry) => entry.update);
  }
}

function validIdentity(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    /^[a-zA-Z0-9_.:-]+$/.test(value)
  );
}

/** Canonical decimal u64 cursor. Conversation cursors need not fit a JavaScript number. */
export function streamCursor(value: string): string {
  if (!/^[0-9]{1,20}$/.test(value) || BigInt(value) > 18446744073709551615n)
    throw new Error(
      "The runtime returned an invalid activity cursor. Reconnect to retry.",
    );
  return BigInt(value).toString();
}
