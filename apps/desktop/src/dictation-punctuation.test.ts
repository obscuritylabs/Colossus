import { describe, expect, it } from "vitest";
import { formatSpokenPunctuation } from "./dictation-punctuation";

describe("spoken punctuation", () => {
  it.each([
    [
      "Check the build period Is it ready question mark",
      "Check the build. Is it ready?",
    ],
    [
      "Check the build. Period. Is it ready, question mark?",
      "Check the build. Is it ready?",
    ],
    ["Yes comma ship it exclamation point.", "Yes, ship it!"],
    ["Options colon fast semi-colon safe full stop", "Options: fast; safe."],
    ["One new line Two new paragraph Three period", "One\nTwo\n\nThree."],
    ["One. New paragraph. Two question mark.", "One.\n\nTwo?"],
    ["new paragraph Heading new line", "\n\nHeading\n"],
    ["Really question mark exclamation mark", "Really?!"],
    ["a literal period of time", "a period of time"],
    [
      "Describe a literal question mark, a literal comma, and a literal semicolon.",
      "Describe a question mark, a comma, and a semicolon.",
    ],
    ["Done period literal period is the word", "Done. period is the word"],
    ["Literal question. Mark", "question Mark"],
    [
      "periodic checks and question_mark remain identifiers",
      "periodic checks and question_mark remain identifiers",
    ],
  ])("formats %j as %j", (speech, expected) => {
    expect(formatSpokenPunctuation(speech).text).toBe(expected);
  });

  it("retains only a bounded suffix while leaving an unfinished command visible", () => {
    const speech = "word ".repeat(1000) + "literal question.";
    const formatted = formatSpokenPunctuation(speech);
    expect(formatted.text).toBe(speech);
    expect(formatted.tail?.raw).toBe("literal question.");
    expect(formatted.text.slice(formatted.tail!.offset)).toBe(
      " literal question.",
    );
    expect(
      formatSpokenPunctuation("question" + ".".repeat(100)).tail,
    ).toBeNull();
  });
});
