import { describe, expect, it } from "vitest";

import {
  LARGE_PASTE_CHAR_THRESHOLD,
  composerDraft,
  editComposerDraft,
  expandComposerDraft,
  pasteIntoComposerDraft,
} from "./composer-paste";

describe("Desktop composer pastes", () => {
  it("keeps smaller pastes visible and condenses text over the TUI threshold", () => {
    const small = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD);
    const first = pasteIntoComposerDraft(composerDraft("Before "), small, 7, 7);
    expect(first.draft.display).toBe(`Before ${small}`);
    expect(first.draft.pastes).toHaveLength(0);

    const large = `${"a".repeat(LARGE_PASTE_CHAR_THRESHOLD)}界`;
    const second = pasteIntoComposerDraft(
      composerDraft("Before after"),
      large,
      7,
      7,
    );
    expect(second.draft.display).toBe(
      "Before [Pasted Content 1001 chars]after",
    );
    expect(expandComposerDraft(second.draft)).toBe(`Before ${large}after`);
  });

  it("preserves Unicode and normalized line endings through multiple same-size pastes", () => {
    const firstText = `# Heading\r\n${"界".repeat(1_000)}`;
    const normalized = firstText.replace(/\r\n/g, "\n");
    const charCount = [...normalized].length;
    const secondText = "b".repeat(charCount);
    const first = pasteIntoComposerDraft(composerDraft("end"), firstText, 0, 0);
    const second = pasteIntoComposerDraft(first.draft, secondText, 0, 0);
    expect(second.draft.display).toContain(
      `[Pasted Content ${charCount} chars #2]`,
    );
    expect(expandComposerDraft(second.draft)).toBe(
      `${secondText}${normalized}end`,
    );
  });

  it("keeps payloads anchored during edits and drops one when its marker is changed", () => {
    const payload = "private text ".repeat(90);
    const first = pasteIntoComposerDraft(composerDraft("after"), payload, 0, 0);
    const withPrefix = editComposerDraft(
      first.draft,
      `Before ${first.draft.display}`,
    );
    expect(expandComposerDraft(withPrefix)).toBe(`Before ${payload}after`);

    const marker = withPrefix.pastes[0]!;
    const edited = editComposerDraft(
      withPrefix,
      withPrefix.display.slice(0, marker.start + 1) +
        "!" +
        withPrefix.display.slice(marker.start + 2),
    );
    expect(edited.pastes).toHaveLength(0);
    expect(expandComposerDraft(edited)).toBe(edited.display);
  });

  it("does not associate a literal or replacement marker with hidden content", () => {
    const payload = "z".repeat(1_001);
    const marker = "[Pasted Content 1001 chars]";
    const initial = composerDraft(`${marker} and `);
    const pasted = pasteIntoComposerDraft(
      initial,
      payload,
      initial.display.length,
      initial.display.length,
    );
    expect(expandComposerDraft(pasted.draft)).toBe(`${marker} and ${payload}`);

    const replaced = pasteIntoComposerDraft(
      pasted.draft,
      marker,
      pasted.draft.pastes[0]!.start,
      pasted.draft.pastes[0]!.end,
    );
    expect(replaced.draft.pastes).toHaveLength(0);
    expect(expandComposerDraft(replaced.draft)).toBe(replaced.draft.display);
  });

  it("drops a marker when deletion is visually ambiguous with adjacent text", () => {
    const payload = "z".repeat(1_001);
    const pasted = pasteIntoComposerDraft(composerDraft("]"), payload, 0, 0);
    const marker = pasted.draft.pastes[0]!;
    const nextDisplay =
      pasted.draft.display.slice(0, marker.end - 1) +
      pasted.draft.display.slice(marker.end);
    const edited = editComposerDraft(pasted.draft, nextDisplay, {
      start: marker.end,
      end: marker.end,
      inputType: "deleteContentBackward",
    });
    expect(edited.pastes).toHaveLength(0);
    expect(expandComposerDraft(edited)).toBe(nextDisplay);
  });
});
