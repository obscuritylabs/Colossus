import { requestResponse } from "./api";

export type TranscriptFormat = "markdown" | "text" | "json";
export const MAX_TRANSCRIPT_DOWNLOAD_BYTES = 64 * 1024 * 1024;
export const MAX_TRANSCRIPT_COPY_BYTES = 2 * 1024 * 1024;
const formats = {
  markdown: { extension: "md", mediaType: "text/markdown" },
  text: { extension: "txt", mediaType: "text/plain" },
  json: { extension: "json", mediaType: "application/json" },
} as const;

function checkAbort(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("Export cancelled", "AbortError");
}

function filename(header: string | null, format: TranscriptFormat): string {
  const fallback = `conversation.${formats[format].extension}`;
  if (!header) return fallback;
  const encoded = /(?:^|;)\s*filename\*=UTF-8''([^;]+)/i.exec(header)?.[1];
  let name = /(?:^|;)\s*filename="([^"]+)"/i.exec(header)?.[1];
  try {
    if (encoded) name = decodeURIComponent(encoded.trim());
  } catch {
    return fallback;
  }
  if (
    !name ||
    new TextEncoder().encode(name).length > 240 ||
    /[\u0000-\u001f\u007f<>:"/\\|?*\u200b-\u200f\u202a-\u202e\u2060-\u206f]/.test(
      name,
    ) ||
    /^[. ]|[. ]$/.test(name) ||
    /^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(name) ||
    !name.endsWith(`.${formats[format].extension}`)
  )
    return fallback;
  return name;
}

/** Obtain a complete server-formatted snapshot; never reconstruct it from loaded history. */
export async function readTranscriptExport(
  threadPath: string,
  format: TranscriptFormat,
  signal: AbortSignal,
  maxBytes: number,
): Promise<{ blob: Blob; filename: string }> {
  checkAbort(signal);
  const response = await requestResponse(
    `${threadPath}/transcript/export`,
    { format },
    signal,
  );
  checkAbort(signal);
  if (
    response.headers
      .get("Content-Type")
      ?.split(";", 1)[0]
      ?.trim()
      .toLowerCase() !== formats[format].mediaType
  ) {
    await response.body?.cancel().catch(() => undefined);
    throw new Error(
      "The server returned an invalid transcript. Nothing was saved or copied.",
    );
  }
  const tooLarge = () =>
    new Error(
      maxBytes === MAX_TRANSCRIPT_COPY_BYTES
        ? "This transcript exceeds the 2 MiB copy limit. Save the transcript instead."
        : "This transcript exceeds the 64 MiB download limit. Use the CLI to export a larger conversation.",
    );
  const length = response.headers.get("Content-Length");
  if (length && (!/^\d+$/.test(length) || Number(length) > maxBytes)) {
    await response.body?.cancel().catch(() => undefined);
    throw tooLarge();
  }
  if (!response.body)
    throw new Error("The transcript response is unavailable.");
  const reader = response.body.getReader();
  const chunks: Uint8Array<ArrayBuffer>[] = [];
  let bytes = 0;
  try {
    for (;;) {
      const next = await reader.read();
      checkAbort(signal);
      if (next.done) break;
      bytes += next.value.byteLength;
      if (bytes > maxBytes) throw tooLarge();
      chunks.push(new Uint8Array(next.value));
    }
    if (!bytes) throw new Error("The server returned an empty transcript.");
    if (length && bytes !== Number(length)) {
      throw new Error(
        "The transcript response was incomplete. Nothing was saved or copied.",
      );
    }
    checkAbort(signal);
    return {
      blob: new Blob(chunks, {
        type: `${formats[format].mediaType};charset=utf-8`,
      }),
      filename: filename(response.headers.get("Content-Disposition"), format),
    };
  } catch (error) {
    await reader.cancel().catch(() => undefined);
    throw error;
  } finally {
    reader.releaseLock();
  }
}

/** The browser acknowledges download initiation, not filesystem persistence. */
export function downloadTranscript(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  try {
    link.href = url;
    link.download = filename;
    document.body.append(link);
    link.click();
  } finally {
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }
}
