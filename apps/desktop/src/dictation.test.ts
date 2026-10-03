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
  it.each([
    [
      "Check the build period Is it ready question mark",
      "Check the build. Is it ready?",
    ],
    ["One new line Two new paragraph Three period", "One\nTwo\n\nThree."],
    ["a literal period of time", "a period of time"],
    ["Keep the literal question mark", "Keep the question mark"],
  ])("formats %j consistently at every word boundary", (speech, expected) => {
    const words = speech.split(" ");
    for (let split = 1; split < words.length; split += 1) {
      let current = transcribeDraft(
        composerDraft(),
        cursor(),
        update(1, 1, words.slice(0, split).join(" "), true),
      );
      current = transcribeDraft(
        current.draft,
        current.cursor,
        update(2, 1, words.slice(split).join(" "), true),
      );
      expect(current.draft.display, `split after word ${split}`).toBe(expected);
    }
  });
  it("restores an owned command prefix if the following segment has an empty final", () => {
    let current = transcribeDraft(
      composerDraft(),
      cursor(),
      update(1, 1, "Ready question", true),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(2, 1, "mark"),
    );
    expect(current.draft.display).toBe("Ready?");
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(2, 2, "", true),
    );
    expect(current.draft.display).toBe("Ready question");
  });
  it("formats revised speech while preserving typed punctuation names and hidden pastes", () => {
    const original = pasteIntoComposerDraft(
      composerDraft("Discuss the period and question mark: "),
      "period question mark ".repeat(100),
      38,
      38,
    ).draft;
    let current = transcribeDraft(
      original,
      cursor(),
      update(1, 1, "Ready period"),
    );
    expect(current.draft.display).toBe(original.display + " Ready.");
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(1, 2, "Ready question mark.", true),
    );
    expect(current.draft.display).toBe(original.display + " Ready?");
    expect(expandComposerDraft(current.draft)).toContain(
      "period question mark ".repeat(100),
    );
    expect(expandComposerDraft(current.draft)).toContain(
      "Discuss the period and question mark:",
    );
  });
  it("joins split commands and replaces their partials without losing a revised word", () => {
    let current = transcribeDraft(
      composerDraft(),
      cursor(),
      update(1, 1, "Is it ready question.", true),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(2, 1, "mark."),
    );
    expect(current.draft.display).toBe("Is it ready?");
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(2, 2, "about the build", true),
    );
    expect(current.draft.display).toBe("Is it ready question. about the build");
  });
  it("does not duplicate model punctuation or remove dictated line breaks across segments", () => {
    let current = transcribeDraft(
      composerDraft(),
      cursor(),
      update(1, 1, "Ready.", true),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(2, 1, "Period. New paragraph", true),
    );
    expect(current.draft.display).toBe("Ready.\n\n");
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(3, 1, "Next line new", true),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(4, 1, "line Last line", true),
    );
    expect(current.draft.display).toBe("Ready.\n\nNext line\nLast line");
  });
  it("can say literal punctuation names even when the escape spans segments", () => {
    let current = transcribeDraft(
      composerDraft("Keep:"),
      cursor(),
      update(1, 1, "literal question", true),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(2, 1, "mark and literal", true),
    );
    current = transcribeDraft(
      current.draft,
      current.cursor,
      update(3, 1, "period", true),
    );
    expect(current.draft.display).toBe("Keep: question mark and period");
  });
  it("leaves an edited or moved settled suffix alone when recording resumes", () => {
    const current = transcribeDraft(
      composerDraft("Keep:"),
      cursor(),
      update(1, 1, "a question", true),
    );
    const edited = transcribeDraft(
      composerDraft("Keep: an answer"),
      current.cursor,
      update(2, 1, "mark", true),
    );
    expect(edited.draft.display).toBe("Keep: an answer mark");
    const appended = transcribeDraft(
      composerDraft(current.draft.display + " typed edits"),
      current.cursor,
      update(2, 1, "mark", true),
    );
    expect(appended.draft.display).toBe("Keep: a question typed edits mark");
  });
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
  it.each([
    ["something", "question mark", "Review this: first?"],
    ["comma", "correction", "Review this: first correction"],
  ])(
    "replaces buffered %j with %j after a failed Send",
    async (partial, final, expected) => {
      const f = fixture();
      await f.controller.start();
      f.events([
        transcript(1, 1, 1, "first", true),
        { type: "boundary", turn_id: 2 },
        transcript(2, 2, 1, partial),
      ]);
      await f.controller.beginSend();
      f.controller.endSend();
      f.events([transcript(2, 2, 2, final, true)]);
      await f.controller.poll();
      expect(f.draft.display).toBe(expected);
    },
  );
  it("can keep punctuation names literal and freezes the setting while recording", async () => {
    const f = fixture();
    f.controller.setSpokenPunctuation(false);
    await f.controller.start();
    f.controller.setSpokenPunctuation(true);
    f.events([transcript(1, 1, 1, "period question mark literal comma", true)]);
    await f.controller.poll();
    expect(f.draft.display).toBe(
      "Review this: period question mark literal comma",
    );
    expect(f.controller.getSnapshot().spokenPunctuation).toBe(false);
  });
  it("keeps split punctuation with the correct draft across successful and failed Sends", async () => {
    for (const accepted of [true, false]) {
      const f = fixture();
      await f.controller.start();
      f.events([
        transcript(1, 1, 1, "first question", true),
        { type: "boundary", turn_id: 2 },
        transcript(2, 2, 1, "Next question", true),
      ]);
      await f.controller.beginSend();
      expect(f.draft.display).toBe("Review this: first question");
      if (accepted) f.replace("");
      f.controller.endSend();
      f.events([transcript(2, 3, 1, "mark.", true)]);
      await f.controller.poll();
      expect(f.draft.display).toBe(
        accepted ? "Next?" : "Review this: first question Next?",
      );
    }
  });
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
