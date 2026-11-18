export type TranscriptFormat = "markdown" | "text" | "json";
export type TranscriptDestination = "save" | "copy";

export interface TranscriptExportProgress {
  phase: "reading" | "choosing_destination" | "saving";
  pagesRead: number;
  messagesRead: number;
}

export interface TranscriptExportResult {
  status: "saved" | "copy" | "copied" | "cancelled";
  messages: number;
  text?: string;
}

export interface TranscriptExportOptions {
  operationId: string;
  format: TranscriptFormat;
  destination: TranscriptDestination;
  signal: AbortSignal;
  onProgress: (progress: TranscriptExportProgress) => void;
}

export interface ConversationExportHost {
  available: boolean;
  scopeKey: string;
  start: (options: TranscriptExportOptions) => Promise<TranscriptExportResult>;
  cancel: (operationId: string) => Promise<void>;
  copy: (text: string, signal: AbortSignal) => Promise<void>;
}

export const MAX_TRANSCRIPT_COPY_BYTES = 2 * 1024 * 1024;

/** Invoke the host clipboard directly from a fresh click, before any await. */
export function copyPreparedTranscript(
  text: string,
  signal: AbortSignal,
  isCurrent: () => boolean,
  writeClipboard: (text: string) => Promise<void>,
): Promise<void> {
  if (signal.aborted) throw new Error("Export cancelled. Nothing was copied.");
  if (!isCurrent())
    throw new Error(
      "The selected conversation or access changed. Nothing was copied.",
    );
  if (new TextEncoder().encode(text).byteLength > MAX_TRANSCRIPT_COPY_BYTES)
    throw new Error(
      "The transcript exceeds the copy limit. Choose Save instead.",
    );
  return writeClipboard(text);
}
