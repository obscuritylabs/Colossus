export const LARGE_PASTE_CHAR_THRESHOLD = 1_000;

export interface PendingComposerPaste {
  placeholder: string;
  text: string;
  start: number;
  end: number;
}

export interface ComposerDraft {
  display: string;
  pastes: readonly PendingComposerPaste[];
}

export interface ComposerEditIntent {
  start: number;
  end: number;
  inputType: string;
}

export function composerDraft(display = ""): ComposerDraft {
  return { display, pastes: [] };
}

function codePointCount(text: string): number {
  let count = 0;
  for (const _character of text) count += 1;
  return count;
}

function pastePlaceholder(draft: ComposerDraft, charCount: number): string {
  const base = `[Pasted Content ${charCount} chars]`;
  const prefix = `[Pasted Content ${charCount} chars #`;
  let maxSuffix = 0;
  for (const paste of draft.pastes) {
    if (paste.placeholder === base) {
      maxSuffix = Math.max(maxSuffix, 1);
    } else if (paste.placeholder.startsWith(prefix)) {
      const suffix = paste.placeholder.slice(prefix.length, -1);
      if (/^[0-9]+$/.test(suffix)) {
        maxSuffix = Math.max(maxSuffix, Number(suffix));
      }
    }
  }
  return maxSuffix === 0 ? base : `${prefix}${maxSuffix + 1}]`;
}

function replaceDraftRange(
  draft: ComposerDraft,
  start: number,
  end: number,
  inserted: string,
): ComposerDraft {
  const from = Math.max(0, Math.min(start, draft.display.length));
  const to = Math.max(from, Math.min(end, draft.display.length));
  const shift = inserted.length - (to - from);
  return {
    display: draft.display.slice(0, from) + inserted + draft.display.slice(to),
    pastes: draft.pastes.flatMap((paste) => {
      if (to <= paste.start) {
        return [
          { ...paste, start: paste.start + shift, end: paste.end + shift },
        ];
      }
      if (from >= paste.end) return [paste];
      // An edited or removed placeholder must never submit its hidden text.
      return [];
    }),
  };
}

/** Reconcile a textarea edit without losing untouched paste placeholders. */
export function editComposerDraft(
  draft: ComposerDraft,
  nextDisplay: string,
  intent?: ComposerEditIntent,
): ComposerDraft {
  const safeDraft =
    intent === undefined
      ? draft
      : {
          ...draft,
          pastes: draft.pastes.filter((paste) => {
            if (intent.end > intent.start) {
              return intent.end <= paste.start || intent.start >= paste.end;
            }
            if (intent.inputType.includes("Backward")) {
              return intent.start <= paste.start || intent.start > paste.end;
            }
            if (intent.inputType.includes("Forward")) {
              return intent.start < paste.start || intent.start >= paste.end;
            }
            return intent.start <= paste.start || intent.start >= paste.end;
          }),
        };
  if (nextDisplay === draft.display) return safeDraft;
  let start = 0;
  while (
    start < draft.display.length &&
    start < nextDisplay.length &&
    draft.display[start] === nextDisplay[start]
  ) {
    start += 1;
  }
  let suffix = 0;
  while (
    suffix < draft.display.length - start &&
    suffix < nextDisplay.length - start &&
    draft.display[draft.display.length - suffix - 1] ===
      nextDisplay[nextDisplay.length - suffix - 1]
  ) {
    suffix += 1;
  }
  return replaceDraftRange(
    safeDraft,
    start,
    draft.display.length - suffix,
    nextDisplay.slice(start, nextDisplay.length - suffix),
  );
}

/** Insert at the textarea selection; native paste is prevented by the caller. */
export function pasteIntoComposerDraft(
  draft: ComposerDraft,
  text: string,
  start: number,
  end: number,
): { draft: ComposerDraft; cursor: number } {
  const normalized = text.replace(/\r\n?/g, "\n");
  const charCount = codePointCount(normalized);
  const from = Math.max(0, Math.min(start, draft.display.length));
  const inserted =
    charCount > LARGE_PASTE_CHAR_THRESHOLD
      ? pastePlaceholder(draft, charCount)
      : normalized;
  const next = replaceDraftRange(draft, from, end, inserted);
  if (charCount <= LARGE_PASTE_CHAR_THRESHOLD) {
    return { draft: next, cursor: from + inserted.length };
  }
  const paste: PendingComposerPaste = {
    placeholder: inserted,
    text: normalized,
    start: from,
    end: from + inserted.length,
  };
  return {
    draft: { ...next, pastes: [...next.pastes, paste] },
    cursor: paste.end,
  };
}

/** The model, queue, and size checks receive the full text in visual order. */
export function expandComposerDraft(draft: ComposerDraft): string {
  const pastes = [...draft.pastes]
    .filter(
      (paste) =>
        draft.display.slice(paste.start, paste.end) === paste.placeholder,
    )
    .sort((first, second) => first.start - second.start);
  let expanded = "";
  let copiedUntil = 0;
  for (const paste of pastes) {
    if (paste.start < copiedUntil) continue;
    expanded += draft.display.slice(copiedUntil, paste.start) + paste.text;
    copiedUntil = paste.end;
  }
  return expanded + draft.display.slice(copiedUntil);
}
