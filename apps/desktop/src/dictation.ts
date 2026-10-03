import { invoke } from "@tauri-apps/api/core";
import {
  editComposerDraft,
  composerDraft,
  expandComposerDraft,
} from "./composer-paste";
import type { ComposerDraft } from "./composer-paste";
import { formatSpokenPunctuation } from "./dictation-punctuation";

export interface TranscriptUpdate {
  segment_id: number;
  revision: number;
  is_final: boolean;
  text: string;
  audio_ms: number;
  inference_ms: number;
}
export type DictationPhase =
  "idle" | "starting" | "recording" | "paused" | "stopped" | "failed";
export type DictationEvent =
  | { type: "state"; phase: "starting" | "recording" | "paused" | "stopped" }
  | { type: "transcript"; turn_id: number; update: TranscriptUpdate }
  | { type: "boundary"; turn_id: number }
  | { type: "failure"; error: string };
export type DictationAction =
  "pause" | "resume" | "finish_turn" | "stop" | "abort";
export interface DictationStatus {
  enabled: boolean;
  model: string | null;
}
export interface DictationApi {
  status(): Promise<DictationStatus>;
  choose(): Promise<DictationStatus>;
  start(): Promise<string>;
  poll(sessionId: string): Promise<DictationEvent[]>;
  control(
    sessionId: string,
    action: DictationAction,
  ): Promise<DictationEvent[]>;
}
export const nativeDictationApi: DictationApi = {
  status: () => invoke("dictation_status"),
  choose: () => invoke("choose_dictation_model"),
  start: () => invoke("start_dictation"),
  poll: (sessionId) => invoke("poll_dictation", { sessionId }),
  control: (sessionId, action) =>
    invoke("control_dictation", { request: { sessionId, action } }),
};

interface DraftCursor {
  segment: number;
  revision: number;
  final: boolean;
  partial: {
    start: number;
    end: number;
    text: string;
    separator: string;
    rawPrefix: string;
  } | null;
  punctuationTail?: {
    start: number;
    end: number;
    text: string;
    raw: string;
  } | null;
}
const freshCursor = (): DraftCursor => ({
  segment: 0,
  revision: 0,
  final: true,
  partial: null,
});

/** Replace one partial in place, preserving settled text and condensed pastes. */
export function transcribeDraft(
  draft: ComposerDraft,
  cursor: DraftCursor,
  update: TranscriptUpdate,
  spokenPunctuation = true,
): { draft: ComposerDraft; cursor: DraftCursor } {
  if (
    !Number.isSafeInteger(update.segment_id) ||
    update.segment_id < 1 ||
    !Number.isSafeInteger(update.revision) ||
    update.revision < 1 ||
    new TextEncoder().encode(update.text).length > 8192
  ) {
    throw new Error(
      "The local transcription returned an invalid update. Recording stopped.",
    );
  }
  if (
    update.segment_id < cursor.segment ||
    (update.segment_id === cursor.segment &&
      (cursor.final || update.revision <= cursor.revision))
  )
    return { draft, cursor };
  if (update.segment_id > cursor.segment && cursor.partial !== null)
    throw new Error(
      "A speech segment was not finalized. Recording stopped to protect your draft.",
    );
  const partial = update.segment_id === cursor.segment ? cursor.partial : null;
  const tail =
    spokenPunctuation &&
    !partial &&
    cursor.punctuationTail?.end === draft.display.length &&
    draft.display.slice(
      cursor.punctuationTail.start,
      cursor.punctuationTail.end,
    ) === cursor.punctuationTail.text
      ? cursor.punctuationTail
      : null;
  const start = partial?.start ?? tail?.start ?? draft.display.length;
  const end = partial?.end ?? tail?.end ?? start;
  if (partial && draft.display.slice(start, end) !== partial.text)
    throw new Error(
      "Your draft changed during transcription. Recording stopped to preserve your edits.",
    );
  const before = draft.display.slice(0, start);
  const separator =
    partial?.separator ?? (before && !/\s$/u.test(before) ? " " : "");
  const rawPrefix = partial?.rawPrefix ?? tail?.raw ?? "";
  const raw = rawPrefix
    ? rawPrefix + (update.text.trim() ? " " + update.text.trim() : "")
    : update.text.trim();
  const formatted = spokenPunctuation
    ? formatSpokenPunctuation(raw)
    : { text: raw, tail: null };
  const join = /^[\n.,?!:;]/u.test(formatted.text) ? "" : separator;
  const text = formatted.text ? join + formatted.text : "";
  const display =
    draft.display.slice(0, start) + text + draft.display.slice(end);
  return {
    draft: editComposerDraft(draft, display),
    cursor: {
      segment: update.segment_id,
      revision: update.revision,
      final: update.is_final,
      partial: update.is_final
        ? null
        : { start, end: start + text.length, text, separator, rawPrefix },
      punctuationTail:
        update.is_final && formatted.tail
          ? {
              start:
                start +
                (formatted.tail.offset === 0
                  ? 0
                  : join.length + formatted.tail.offset),
              end: start + text.length,
              text:
                formatted.tail.offset === 0
                  ? text
                  : formatted.text.slice(formatted.tail.offset),
              raw: formatted.tail.raw,
            }
          : null,
    },
  };
}

const FAILURE_MESSAGES: Record<string, string> = {
  model_unavailable: "Choose a local Whisper model before recording.",
  model_integrity:
    "The model failed verification. Download the pinned model again and choose it.",
  model_unsupported: "Choose the pinned tiny.en or base.en Whisper model.",
  microphone_missing:
    "No microphone was found. Connect one and check your default input device.",
  capture_unavailable:
    "Microphone access failed. Allow Colossus in your system microphone privacy settings and check the input device, then try again.",
  capture_unsupported:
    "This microphone format is unsupported. Try a different input device.",
  capture_overrun:
    "Transcription could not keep up. Recording stopped; try the tiny.en model.",
  inference:
    "Local transcription failed or timed out. Recording stopped. Your draft is kept.",
  transcript_limit:
    "A speech segment exceeded the size limit. Recording stopped. Your draft is kept.",
};

export interface DictationSnapshot extends DictationStatus {
  spokenPunctuation: boolean;
  phase: DictationPhase;
  sessionId: string | null;
  busy: boolean;
  sending: boolean;
  error: string;
}
interface DraftPort {
  read(): ComposerDraft;
  write(draft: ComposerDraft): void;
}

/** Owns ordered native replies and the current/next draft boundary. */
export class DictationController {
  private snapshot: DictationSnapshot = {
    enabled: false,
    model: null,
    spokenPunctuation: true,
    phase: "idle",
    sessionId: null,
    busy: false,
    sending: false,
    error: "",
  };
  private listeners = new Set<() => void>();
  private generation = 0;
  private queue: Promise<unknown> = Promise.resolve();
  private cursor = freshCursor();
  private turn = 1;
  private sendTurn: number | null = null;
  private pending = { draft: composerDraft(), cursor: freshCursor() };
  constructor(
    private readonly api: DictationApi,
    private readonly drafts: DraftPort,
    private readonly byteLimit = 65_536,
  ) {}
  getSnapshot = (): DictationSnapshot => this.snapshot;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  setSpokenPunctuation(enabled: boolean) {
    if (this.snapshot.sessionId || this.snapshot.busy || this.snapshot.sending)
      return;
    this.change({ spokenPunctuation: enabled });
  }
  private change(next: Partial<DictationSnapshot>) {
    this.snapshot = { ...this.snapshot, ...next };
    this.listeners.forEach((listener) => listener());
  }
  private serialized<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.queue.then(operation);
    this.queue = result.catch(() => {});
    return result;
  }
  private message(error: unknown): string {
    if (error instanceof Error) return error.message;
    if (
      error &&
      typeof error === "object" &&
      "message" in error &&
      typeof error.message === "string"
    )
      return error.message;
    return "Offline dictation is unavailable. Your draft is kept.";
  }
  private async fail(error: unknown) {
    const id = this.snapshot.sessionId;
    this.change({
      phase: "failed",
      error: this.message(error),
      busy: false,
      sessionId: null,
    });
    if (id) {
      try {
        await this.api.control(id, "abort");
      } catch {
        /* Native failure already releases capture. */
      }
    }
  }
  private apply(events: DictationEvent[]) {
    for (const event of events) {
      if (event.type === "state") {
        this.change({
          phase: event.phase,
          ...(event.phase === "stopped" ? { sessionId: null } : {}),
        });
      } else if (event.type === "failure") {
        throw new Error(
          FAILURE_MESSAGES[event.error] ??
            "Offline transcription stopped. Your draft is kept.",
        );
      } else if (event.type === "boundary") {
        if (event.turn_id !== this.turn + 1)
          throw new Error(
            "The recording boundary is invalid. Recording stopped.",
          );
        this.turn = event.turn_id;
        if (this.sendTurn === null) this.cursor = freshCursor();
      } else {
        if (event.turn_id !== this.turn)
          throw new Error(
            "A transcript arrived for an unexpected draft. Recording stopped.",
          );
        const pending = this.sendTurn !== null && event.turn_id > this.sendTurn;
        const current = pending
          ? this.pending
          : { draft: this.drafts.read(), cursor: this.cursor };
        const next = transcribeDraft(
          current.draft,
          current.cursor,
          event.update,
          this.snapshot.spokenPunctuation,
        );
        if (
          new TextEncoder().encode(expandComposerDraft(next.draft)).length >
          this.byteLimit
        )
          throw new Error(
            "Your draft reached the text limit. Recording stopped; send or shorten the draft before continuing.",
          );
        if (pending) this.pending = next;
        else {
          this.cursor = next.cursor;
          this.drafts.write(next.draft);
        }
      }
    }
  }
  async inspect() {
    try {
      this.change(await this.serialized(() => this.api.status()));
    } catch (error) {
      this.change({ error: this.message(error) });
    }
  }
  async choose() {
    this.change({ busy: true, error: "" });
    try {
      this.change(await this.serialized(() => this.api.choose()));
    } catch (error) {
      this.change({ error: this.message(error) });
    } finally {
      this.change({ busy: false });
    }
  }
  async start() {
    if (this.snapshot.busy || this.snapshot.sessionId) return;
    const generation = this.generation;
    this.change({ busy: true, error: "", phase: "starting" });
    try {
      const id = await this.serialized(() => this.api.start());
      if (generation !== this.generation) {
        await this.api.control(id, "abort");
        return;
      }
      this.cursor = freshCursor();
      this.turn = 1;
      this.change({ sessionId: id });
    } catch (error) {
      if (generation === this.generation) await this.fail(error);
    } finally {
      if (generation === this.generation) this.change({ busy: false });
    }
  }
  async poll() {
    const id = this.snapshot.sessionId;
    if (!id || this.snapshot.busy) return;
    const generation = this.generation;
    try {
      await this.serialized(async () => {
        if (generation !== this.generation || id !== this.snapshot.sessionId)
          return;
        const events = await this.api.poll(id);
        if (generation === this.generation) this.apply(events);
      });
    } catch (error) {
      if (generation === this.generation) await this.fail(error);
    }
  }
  async control(action: "pause" | "resume" | "stop") {
    const id = this.snapshot.sessionId;
    if (!id || this.snapshot.busy) return;
    const generation = this.generation;
    this.change({ busy: true, error: "" });
    try {
      await this.serialized(async () => {
        const events = await this.api.control(id, action);
        if (generation === this.generation) this.apply(events);
      });
    } catch (error) {
      if (generation === this.generation) await this.fail(error);
    } finally {
      if (generation === this.generation) this.change({ busy: false });
    }
  }
  async beginSend(): Promise<boolean> {
    if (this.snapshot.sending || this.snapshot.busy) return false;
    this.change({ sending: true });
    this.sendTurn = this.turn;
    this.pending = { draft: composerDraft(), cursor: freshCursor() };
    const id = this.snapshot.sessionId;
    if (!id) return true;
    const generation = this.generation;
    try {
      await this.serialized(async () => {
        const events = await this.api.control(id, "finish_turn");
        if (generation === this.generation) this.apply(events);
      });
      return generation === this.generation && this.snapshot.phase !== "failed";
    } catch (error) {
      if (generation === this.generation) await this.fail(error);
      return false;
    }
  }
  endSend() {
    if (this.sendTurn === null) return;
    const draft = this.drafts.read();
    const baseSeparator =
      draft.display && !/\s$/u.test(draft.display) ? " " : "";
    const separator =
      this.pending.draft.display &&
      !/^[\n.,?!:;]/u.test(this.pending.draft.display)
        ? baseSeparator
        : "";
    const offset = draft.display.length + separator.length;
    if (this.pending.draft.display)
      this.drafts.write(
        editComposerDraft(
          draft,
          draft.display + separator + this.pending.draft.display,
        ),
      );
    this.cursor = {
      ...this.pending.cursor,
      partial: this.pending.cursor.partial
        ? {
            ...this.pending.cursor.partial,
            start:
              this.pending.cursor.partial.start === 0
                ? draft.display.length
                : this.pending.cursor.partial.start + offset,
            end: this.pending.cursor.partial.end + offset,
            text:
              (this.pending.cursor.partial.start === 0 ? separator : "") +
              this.pending.cursor.partial.text,
            separator:
              this.pending.cursor.partial.start === 0
                ? baseSeparator
                : this.pending.cursor.partial.separator,
          }
        : null,
      punctuationTail: this.pending.cursor.punctuationTail
        ? {
            ...this.pending.cursor.punctuationTail,
            start:
              this.pending.cursor.punctuationTail.start === 0
                ? draft.display.length
                : this.pending.cursor.punctuationTail.start + offset,
            end: this.pending.cursor.punctuationTail.end + offset,
            text:
              (this.pending.cursor.punctuationTail.start === 0
                ? separator
                : "") + this.pending.cursor.punctuationTail.text,
          }
        : null,
    };
    this.sendTurn = null;
    this.pending = { draft: composerDraft(), cursor: freshCursor() };
    this.change({ sending: false });
  }
  /** Stop on thread/workspace navigation and ignore replies from the old session. */
  reset() {
    const id = this.snapshot.sessionId;
    this.generation += 1;
    this.cursor = freshCursor();
    this.turn = 1;
    this.sendTurn = null;
    this.pending = { draft: composerDraft(), cursor: freshCursor() };
    this.change({
      sessionId: null,
      phase: "idle",
      busy: false,
      sending: false,
      error: "",
    });
    if (id) void this.api.control(id, "abort").catch(() => {});
  }
}
