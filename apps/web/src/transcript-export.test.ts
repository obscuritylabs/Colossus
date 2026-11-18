import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiFailure } from "./api";
import {
  MAX_TRANSCRIPT_COPY_BYTES,
  readTranscriptExport,
} from "./transcript-export";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
const path = "/api/projects/p/threads/t";
function exported(
  body: BodyInit = "# Conversation\n\nComplete server snapshot.\n",
  headers: HeadersInit = {},
) {
  return new Response(body, {
    headers: {
      "Content-Type": "text/markdown;charset=utf-8",
      "Content-Disposition":
        "attachment; filename*=UTF-8''%E6%9D%B1%E4%BA%AC-id.md",
      ...headers,
    },
  });
}

describe("complete server transcript transport", () => {
  it("uses ordinary same-origin CSRF authority and exact server bytes/filename", async () => {
    const fetch = vi.fn().mockResolvedValue(exported());
    vi.stubGlobal("fetch", fetch);
    const controller = new AbortController();
    const result = await readTranscriptExport(
      path,
      "markdown",
      controller.signal,
      MAX_TRANSCRIPT_COPY_BYTES,
    );
    expect(fetch).toHaveBeenCalledWith(`${path}/transcript/export`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-Colossus-CSRF": "1" },
      body: '{"format":"markdown"}',
      signal: controller.signal,
    });
    expect(result.filename).toBe("東京-id.md");
    expect(await result.blob.text()).toBe(
      "# Conversation\n\nComplete server snapshot.\n",
    );
  });
  it("supports text and JSON without parsing or reserializing released bytes", async () => {
    for (const [format, contentType, bytes] of [
      ["text", "text/plain", "User — 東京\nraw text\n"],
      ["json", "application/json", '{"schema_version":1,"complete":true}\n'],
    ] as const) {
      vi.stubGlobal(
        "fetch",
        vi
          .fn()
          .mockResolvedValue(
            new Response(bytes, { headers: { "Content-Type": contentType } }),
          ),
      );
      const result = await readTranscriptExport(
        path,
        format,
        new AbortController().signal,
        4096,
      );
      expect(await result.blob.text()).toBe(bytes);
      expect(result.filename).toBe(
        format === "text" ? "conversation.txt" : "conversation.json",
      );
    }
  });
  it("rejects wrong media types, unsafe filenames and incomplete or oversized responses", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          exported("private fallback HTML", { "Content-Type": "text/html" }),
        ),
    );
    await expect(
      readTranscriptExport(path, "markdown", new AbortController().signal, 100),
    ).rejects.toThrow("invalid transcript");
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          exported("complete", {
            "Content-Disposition":
              "attachment; filename*=UTF-8''..%2Fprivate.md",
          }),
        ),
    );
    expect(
      (
        await readTranscriptExport(
          path,
          "markdown",
          new AbortController().signal,
          100,
        )
      ).filename,
    ).toBe("conversation.md");
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          exported("small", {
            "Content-Length": String(MAX_TRANSCRIPT_COPY_BYTES + 1),
          }),
        ),
    );
    await expect(
      readTranscriptExport(
        path,
        "markdown",
        new AbortController().signal,
        MAX_TRANSCRIPT_COPY_BYTES,
      ),
    ).rejects.toThrow("2 MiB copy limit");
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(exported("too long")));
    await expect(
      readTranscriptExport(path, "markdown", new AbortController().signal, 2),
    ).rejects.toThrow("download limit");
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("partial"));
        controller.error(new Error("transport closed"));
      },
    });
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          new Response(stream, {
            headers: { "Content-Type": "text/markdown" },
          }),
        ),
    );
    await expect(
      readTranscriptExport(path, "markdown", new AbortController().signal, 100),
    ).rejects.toThrow("transport closed");
  });
  it("reconciles denied access and refuses late bytes after cancellation", async () => {
    const dispatchEvent = vi.fn();
    vi.stubGlobal("window", { dispatchEvent });
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          new Response('{"error":"permission_denied"}', { status: 403 }),
        ),
    );
    await expect(
      readTranscriptExport(path, "markdown", new AbortController().signal, 100),
    ).rejects.toBeInstanceOf(ApiFailure);
    expect(dispatchEvent.mock.calls[0]?.[0].type).toBe(
      "colossus:web:reconcile-identity",
    );
    let resolve!: (response: Response) => void;
    vi.stubGlobal(
      "fetch",
      vi.fn(
        () =>
          new Promise<Response>((done) => {
            resolve = done;
          }),
      ),
    );
    const controller = new AbortController();
    const pending = readTranscriptExport(
      path,
      "markdown",
      controller.signal,
      100,
    );
    controller.abort();
    resolve(exported());
    await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  });
});
