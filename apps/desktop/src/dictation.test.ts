import { describe, expect, it, vi } from "vitest";
import {
  composerDraft,
  expandComposerDraft,
  pasteIntoComposerDraft,
} from "./composer-paste";
import { DictationController, transcribeDraft } from "./dictation";
import type {
  DictationApi,
  DictationEvent,
  TranscriptUpdate,
} from "./dictation";

const cursor = (): ReturnType<typeof transcribeDraft>["cursor"] => ({
  segment: 0,
  revision: 0,
  final: true,
  partial: null,
});
const update = (
  segment: number,
  revision: number,
  text: string,
  final = false,
): TranscriptUpdate => ({
  segment_id: segment,
  revision,
  text,
  is_final: final,
  audio_ms: 2000,
  inference_ms: 20,
});
const transcript = (
  turn: number,
  segment: number,
  revision: number,
  text: string,
  final = false,
): DictationEvent => ({
  type: "transcript",
  turn_id: turn,
  update: update(segment, revision, text, final),
});

function fixture(byteLimit = 65_536) {
  let draft = composerDraft("Review this:");
  let events: DictationEvent[] = [];
  const api: DictationApi = {
    status: vi.fn(async () => ({ enabled: true, model: "Tiny English" })),
    choose: vi.fn(async () => ({ enabled: true, model: "Tiny English" })),
    start: vi.fn(async () => "session-one"),
    poll: vi.fn(async () => events.splice(0)),
    control: vi.fn(async () => events.splice(0)),
  };
  const controller = new DictationController(
    api,
    {
      read: () => draft,
      write: (next) => {
        draft = next;
      },
    },
    byteLimit,
  );
  return {
    api,
    controller,
    get draft() {
      return draft;
    },
    replace: (text: string) => {
      draft = composerDraft(text);
    },
    events: (next: DictationEvent[]) => {
      events = next;
    },
  };
}

describe("dictation draft revisions", () => {
  it("settles a long session in order, replacing partials and rejecting stale revisions", () => {
    let current = { draft: composerDraft(), cursor: cursor() };
    const words: string[] = [];
    for (let segment = 1; segment <= 300; segment += 1) {
      const word = `segment-${segment}`;
      current = transcribeDraft(
        current.draft,
        current.cursor,
        update(segment, 1, "incomplete"),
      );
      current = transcribeDraft(
        current.draft,
        current.cursor,
        update(segment, 2, word, true),
      );
      current = transcribeDraft(
        current.draft,
        current.cursor,
        update(segment, 1, "duplicate"),
      );
      words.push(word);
    }
    expect(current.draft.display).toBe(words.join(" "));
  });
  it("clears an empty final and retains condensed paste content and existing edits", () => {
    const original = pasteIntoComposerDraft(
      composerDraft("User edits: "),
      "long input ".repeat(100),
      12,
      12,
    ).draft;
    let current = transcribeDraft(
      original,
      cursor(),
      update(1, 1, "hallucinated silence"),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(1, 2, "", true),
    );
    expect(current.draft).toEqual(original);
    expect(expandComposerDraft(current.draft)).toContain(
      "long input ".repeat(100),
    );
  });
  it("stops safely if an unsettled span was edited or its final never arrived", () => {
    const current = transcribeDraft(
      composerDraft("Keep edits:"),
      cursor(),
      update(1, 1, "speech"),
    );
    expect(() =>
      transcribeDraft(
        composerDraft("Keep edits: correction"),
        current.cursor,
        update(1, 2, "speech revised", true),
      ),
    ).toThrow("preserve your edits");
    expect(() =>
      transcribeDraft(current.draft, current.cursor, update(2, 1, "later")),
    ).toThrow("not finalized");
  });
});

describe("native dictation composer boundary", () => {
  it("can stop the microphone while a submitted message is still pending", async () => {
    const f = fixture();
    await f.controller.start();
    f.events([
      transcript(1, 1, 1, "first", true),
      { type: "boundary", turn_id: 2 },
      transcript(2, 2, 1, "next"),
    ]);
    await f.controller.beginSend();
    f.events([
      transcript(2, 2, 2, "next finalized", true),
      { type: "state", phase: "stopped" },
    ]);
    await f.controller.control("stop");
    expect(f.controller.getSnapshot().sending).toBe(true);
    expect(f.controller.getSnapshot().sessionId).toBeNull();
    f.replace("");
    f.controller.endSend();
    expect(f.draft.display).toBe("next finalized");
    expect(f.api.control).toHaveBeenCalledWith("session-one", "stop");
  });
  it("finalizes before Send, buffers the next turn, and keeps the microphone session active", async () => {
    const f = fixture();
    await f.controller.start();
    f.events([
      { type: "state", phase: "recording" },
      transcript(1, 1, 1, "first partial"),
    ]);
    await f.controller.poll();
    f.events([
      transcript(1, 1, 2, "first final", true),
      { type: "boundary", turn_id: 2 },
      transcript(2, 2, 1, "next partial"),
    ]);
    expect(await f.controller.beginSend()).toBe(true);
    expect(f.draft.display).toBe("Review this: first final");
    f.replace(""); // Existing run path accepted this draft.
    f.controller.endSend();
    expect(f.draft.display).toBe("next partial");
    f.events([transcript(2, 2, 2, "next final", true)]);
    await f.controller.poll();
    expect(f.draft.display).toBe("next final");
    expect(f.controller.getSnapshot().sessionId).toBe("session-one");
    expect(f.api.control).toHaveBeenCalledWith("session-one", "finish_turn");
    expect(f.api.start).toHaveBeenCalledTimes(1);
  });
  it("preserves a failed submission and appends speech recorded while it was sending", async () => {
    const f = fixture();
    await f.controller.start();
    f.events([
      transcript(1, 1, 1, "first", true),
      { type: "boundary", turn_id: 2 },
    ]);
    await f.controller.beginSend();
    f.events([transcript(2, 2, 1, "continued")]);
    await f.controller.poll();
    expect(f.draft.display).toBe("Review this: first");
    f.controller.endSend();
    expect(f.draft.display).toBe("Review this: first continued");
    f.events([transcript(2, 2, 2, "continued speech", true)]);
    await f.controller.poll();
    expect(f.draft.display).toBe("Review this: first continued speech");
  });
  it("ignores a late reply after navigation and cancels the old microphone session", async () => {
    const f = fixture();
    await f.controller.start();
    let reply!: (events: DictationEvent[]) => void;
    vi.mocked(f.api.poll).mockImplementation(
      () =>
        new Promise((resolve) => {
          reply = resolve;
        }),
    );
    const pending = f.controller.poll();
    await Promise.resolve();
    await Promise.resolve();
    f.controller.reset();
    f.replace("New thread");
    reply([transcript(1, 1, 1, "old private speech", true)]);
    await pending;
    expect(f.draft.display).toBe("New thread");
    expect(f.api.control).toHaveBeenCalledWith("session-one", "abort");
  });
  it("does not authorize Send on inference failure and retains the latest visible draft", async () => {
    const f = fixture();
    await f.controller.start();
    f.events([transcript(1, 1, 1, "visible partial")]);
    await f.controller.poll();
    f.events([{ type: "failure", error: "inference" }]);
    expect(await f.controller.beginSend()).toBe(false);
    f.controller.endSend();
    expect(f.draft.display).toBe("Review this: visible partial");
    expect(f.controller.getSnapshot().phase).toBe("failed");
    expect(f.api.control).toHaveBeenCalledWith("session-one", "abort");
  });
  it("stops capture at the composer byte limit without silently dropping the prior draft", async () => {
    const f = fixture(24);
    await f.controller.start();
    f.events([transcript(1, 1, 1, "fits")]);
    await f.controller.poll();
    const previous = f.draft;
    f.events([transcript(1, 1, 2, "this is too long to fit", true)]);
    await f.controller.poll();
    expect(f.draft).toEqual(previous);
    expect(f.controller.getSnapshot().error).toContain("text limit");
    expect(f.api.control).toHaveBeenCalledWith("session-one", "abort");
  });
});
