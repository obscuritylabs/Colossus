const PUNCTUATION: Readonly<Record<string, string>> = {
  period: ".",
  "full stop": ".",
  comma: ",",
  "question mark": "?",
  "exclamation mark": "!",
  "exclamation point": "!",
  colon: ":",
  semicolon: ";",
  "semi colon": ";",
  "semi-colon": ";",
  "new line": "\n",
  "new paragraph": "\n\n",
};
// Whisper sometimes inserts punctuation between the words of a command.
const WORD_GAP = "(?:[ \\t]+|[.,?!:;][ \\t]*)";
const COMMAND = new RegExp(
  `\\b(literal${WORD_GAP})?(${Object.keys(PUNCTUATION)
    .map((name) => name.replaceAll(" ", WORD_GAP))
    .join("|")})\\b`,
  "giu",
);
const COMMAND_PREFIX =
  /\b(literal(?:[ \t.,?!:;]+(?:question|exclamation|full|new|semi))?|question|exclamation|full|new|semi)[ \t]*[.,?!:;]*$/iu;

export interface PunctuationTail {
  raw: string;
  offset: number;
}
export interface FormattedDictation {
  text: string;
  tail: PunctuationTail | null;
}

function appendProse(
  text: string,
  prose: string,
  afterCommand: boolean,
): string {
  if (!afterCommand) return text + prose;
  const next = prose.replace(/^[ \t]+/u, "");
  const separator =
    next && !/\s$/u.test(text) && !/^[\n.,?!:;]/u.test(next) ? " " : "";
  return text + separator + next;
}

/** Format only recognized speech, retaining a bounded suffix for split commands. */
export function formatSpokenPunctuation(source: string): FormattedDictation {
  const raw = source.trim();
  let text = "";
  let copied = 0;
  let afterCommand = false;
  for (const match of raw.matchAll(COMMAND)) {
    const literal = Boolean(match[1]);
    const name = match[2]!.replace(/[ \t.,?!:;]+/gu, " ");
    const punctuation = PUNCTUATION[name.toLowerCase()]!;
    let prose = raw.slice(copied, match.index);
    if (!literal && !punctuation.startsWith("\n")) {
      // Replace auto punctuation immediately before a spoken command; do not
      // collapse punctuation elsewhere in the recognized sentence.
      prose = prose.replace(/[ \t]*[.,?!:;]+[ \t]*$/u, "");
    }
    text = appendProse(text, prose, afterCommand);
    text = literal
      ? appendProse(text, name, afterCommand)
      : text.replace(/[ \t]+$/u, "") + punctuation;
    afterCommand = !literal;
    copied = match.index + match[0].length;
    if (!literal) {
      // A model-emitted full stop after "question mark" must not become "?.".
      copied += raw.slice(copied).match(/^[ \t]*[.,?!:;]+/u)?.[0].length ?? 0;
    }
  }
  const remainder = raw.slice(copied);
  text = appendProse(text, remainder, afterCommand).replace(
    /^[ \t]+|[ \t]+$/gu,
    "",
  );

  // Keep unfinished multiword commands visible. Only this owned suffix can be
  // revised when the next segment supplies "mark", "line", "paragraph", etc.
  const prefix = raw.match(COMMAND_PREFIX)?.[0];
  const nativePunctuation =
    !afterCommand || remainder.trim()
      ? raw.match(/[.,?!:;]+$/u)?.[0]
      : undefined;
  const suffix = prefix ?? nativePunctuation;
  if (!suffix || suffix.length > 64 || !text.endsWith(suffix))
    return { text, tail: null };
  let offset = text.length - suffix.length;
  while (offset > 0 && /[ \t]/u.test(text[offset - 1]!)) offset -= 1;
  return { text, tail: { raw: suffix, offset } };
}
