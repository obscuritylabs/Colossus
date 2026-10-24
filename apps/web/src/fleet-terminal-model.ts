import type { ITheme, ITerminalOptions } from "@xterm/xterm";
import type { HumanTerminalFrame } from "@colossus/control-plane-client";

export const MAX_TERMINAL_INPUT = 64 * 1024;
export const MAX_PENDING_INPUT = 128 * 1024;
export const MAX_TERMINAL_COPY = 256 * 1024;

export function terminalBytes(data: string): Uint8Array {
  if (data.length > 87384 || !/^[A-Za-z0-9+/]*={0,2}$/u.test(data))
    throw new Error("The workspace returned invalid terminal output.");
  const decoded = atob(data);
  if (decoded.length > 65536)
    throw new Error("The workspace returned too much terminal output.");
  return Uint8Array.from(decoded, (character) => character.charCodeAt(0));
}

export function terminalBase64(bytes: Uint8Array) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export function terminalSequence(value: number) {
  if (!Number.isSafeInteger(value) || value < 0)
    throw new Error("The workspace returned an invalid terminal cursor.");
  return value;
}

export function validateTerminalFrames(
  frames: HumanTerminalFrame[],
  after: number,
) {
  if (!Array.isArray(frames) || frames.length > 1024)
    throw new Error("The workspace returned an invalid terminal history.");
  let bytes = 0;
  for (const frame of frames) {
    if (terminalSequence(frame.sequence) !== after + 1)
      throw new Error(
        "Terminal output has a gap. Reconnect to review its state.",
      );
    after = frame.sequence;
    if (frame.kind.kind === "output")
      bytes += terminalBytes(frame.kind.data_base64).byteLength;
    else if (
      frame.kind.kind !== "resize" ||
      !Number.isInteger(frame.kind.rows) ||
      !Number.isInteger(frame.kind.cols) ||
      frame.kind.rows < 2 ||
      frame.kind.rows > 512 ||
      frame.kind.cols < 2 ||
      frame.kind.cols > 512
    )
      throw new Error("The workspace returned invalid terminal dimensions.");
    if (bytes > 65536)
      throw new Error("The workspace exceeded the terminal read limit.");
  }
  return after;
}

/** Resolve shared tokens through CSS so every supported palette/text size applies. */
export function terminalPresentation(element: HTMLElement): ITerminalOptions {
  const style = getComputedStyle(element);
  const token = (name: string) => style.getPropertyValue(name).trim();
  const color = (name: string) => token(name);
  const theme: ITheme = {
    background: color("--surface-inset"),
    foreground: color("--text"),
    cursor: color("--text-strong"),
    cursorAccent: color("--surface-inset"),
    selectionBackground: color("--surface-selected-hover"),
    black: color("--surface-inset"),
    red: color("--red"),
    green: color("--green"),
    yellow: color("--amber"),
    blue: color("--blue"),
    magenta: color("--purple"),
    cyan: color("--cyan"),
    white: color("--text"),
    brightBlack: color("--muted"),
    brightRed: color("--red-text"),
    brightGreen: color("--green-text"),
    brightYellow: color("--amber-text"),
    brightBlue: color("--accent-text"),
    brightMagenta: color("--purple-text"),
    brightCyan: color("--cyan"),
    brightWhite: color("--text-strong"),
  };
  return {
    theme,
    fontFamily: token("--font-mono"),
    fontSize: Number.parseFloat(style.fontSize),
    lineHeight:
      Number.parseFloat(style.lineHeight) / Number.parseFloat(style.fontSize),
    cursorBlink: true,
    scrollback: 5000,
    screenReaderMode: false,
    minimumContrastRatio: 4.5,
    allowProposedApi: false,
    allowTransparency: false,
    convertEol: false,
    windowOptions: {},
    linkHandler: { activate: () => undefined },
  };
}
