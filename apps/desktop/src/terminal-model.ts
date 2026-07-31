import type { TerminalContext, TerminalPlanContext } from "./types";

const MAX_TERMINAL_WRITE_CHUNK_BYTES = 48 * 1024;
const MAX_TERMINAL_INPUT_EVENT_BYTES = 256 * 1024;
const MAX_TERMINAL_OUTPUT_CHUNK_BYTES = 64 * 1024;
const MAX_TERMINAL_OUTPUT_BASE64_CHARACTERS =
  Math.ceil(MAX_TERMINAL_OUTPUT_CHUNK_BYTES / 3) * 4;
const MIN_TERMINAL_DIMENSION = 2;
const MAX_TERMINAL_DIMENSION = 512;
const DEFAULT_TERMINAL_COLS = 80;
const DEFAULT_TERMINAL_ROWS = 24;

export interface TerminalContentInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

export function terminalContextChanged(
  previous: TerminalContext | null,
  next: TerminalContext,
): boolean {
  return (
    previous === null ||
    previous.contextGeneration !== next.contextGeneration ||
    previous.enabled !== next.enabled ||
    previous.workspaceId !== next.workspaceId
  );
}

export function terminalLaunchRequested(
  _previous: TerminalContext | null,
  next: TerminalContext,
): boolean {
  // Native consumes launch intent atomically and returns `requestedKind` exactly
  // once. Do not let a prior, non-claiming context response suppress that one-shot.
  return next.requestedKind !== null;
}

export function terminalPlanSelectionInputs(
  context: TerminalPlanContext,
): readonly string[] {
  return [
    `/session resume ${context.sessionId}\r`,
    `/plan use ${context.planId}\r`,
  ];
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

export function terminalInputChunks(value: string): string[] {
  const encoded = new TextEncoder().encode(value);
  if (encoded.length > MAX_TERMINAL_INPUT_EVENT_BYTES) {
    throw new Error("Terminal input event exceeds the renderer limit.");
  }
  const chunks: string[] = [];
  for (
    let offset = 0;
    offset < encoded.length;
    offset += MAX_TERMINAL_WRITE_CHUNK_BYTES
  ) {
    chunks.push(
      bytesToBase64(
        encoded.slice(offset, offset + MAX_TERMINAL_WRITE_CHUNK_BYTES),
      ),
    );
  }
  return chunks;
}

export function decodeTerminalOutput(value: string): Uint8Array {
  if (value.length > MAX_TERMINAL_OUTPUT_BASE64_CHARACTERS) {
    throw new Error("Terminal output event exceeds the renderer limit.");
  }
  const binary = atob(value);
  if (binary.length > MAX_TERMINAL_OUTPUT_CHUNK_BYTES) {
    throw new Error("Terminal output event exceeds the renderer limit.");
  }
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

export function terminalDimensions(width: number, height: number) {
  return {
    cols: Math.max(
      MIN_TERMINAL_DIMENSION,
      Math.min(MAX_TERMINAL_DIMENSION, Math.floor(width / 8.4)),
    ),
    rows: Math.max(
      MIN_TERMINAL_DIMENSION,
      Math.min(MAX_TERMINAL_DIMENSION, Math.floor(height / 17.2)),
    ),
  };
}

export function terminalContentDimensions(
  width: number,
  height: number,
  insets: TerminalContentInsets,
) {
  const contentWidth = Math.max(0, width - insets.left - insets.right);
  const contentHeight = Math.max(0, height - insets.top - insets.bottom);
  return terminalDimensions(contentWidth, contentHeight);
}

export function terminalOpenDimensions(width: number, height: number) {
  if (width <= 0 || height <= 0) {
    return {
      cols: DEFAULT_TERMINAL_COLS,
      rows: DEFAULT_TERMINAL_ROWS,
    };
  }
  return terminalDimensions(width, height);
}
