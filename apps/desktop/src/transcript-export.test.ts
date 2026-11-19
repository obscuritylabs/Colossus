import { describe, expect, it, vi } from "vitest";
import {
  MAX_TRANSCRIPT_COPY_BYTES,
  copyPreparedTranscript,
} from "./transcript-export";

describe("prepared transcript clipboard", () => {
  it("writes synchronously in a fresh click after a delayed preparation", async () => {
    let release!: (text: string) => void;
    const native = new Promise<string>((resolve) => {
      release = resolve;
    });
    let inClick = false;
    const write = vi.fn((text: string) => {
      if (!inClick)
        throw new Error(
          "NotAllowedError: clipboard requires the current click",
        );
      expect(text).toBe("Released complete transcript");
      return Promise.resolve();
    });
    release("Released complete transcript");
    const text = await native;
    expect(write).not.toHaveBeenCalled();
    inClick = true;
    const writing = copyPreparedTranscript(
      text,
      new AbortController().signal,
      () => true,
      write,
    );
    inClick = false;
    expect(write).toHaveBeenCalledExactlyOnceWith(text);
    await writing;
  });

  it("refuses a cancelled prepared snapshot before touching the clipboard", () => {
    const cancellation = new AbortController();
    cancellation.abort();
    const write = vi.fn();
    expect(() =>
      copyPreparedTranscript(
        "released",
        cancellation.signal,
        () => true,
        write,
      ),
    ).toThrow("cancelled");
    expect(write).not.toHaveBeenCalled();
  });

  it("rechecks source identity and access synchronously before copying", () => {
    const write = vi.fn();
    expect(() =>
      copyPreparedTranscript(
        "released",
        new AbortController().signal,
        () => false,
        write,
      ),
    ).toThrow("conversation or access changed");
    expect(write).not.toHaveBeenCalled();
  });

  it("counts UTF-8 bytes against the bounded released text limit", () => {
    const write = vi.fn();
    expect(() =>
      copyPreparedTranscript(
        "é".repeat(MAX_TRANSCRIPT_COPY_BYTES / 2 + 1),
        new AbortController().signal,
        () => true,
        write,
      ),
    ).toThrow("copy limit");
    expect(write).not.toHaveBeenCalled();
  });
});
