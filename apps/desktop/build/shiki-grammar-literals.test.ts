import { readFileSync, readdirSync } from "node:fs";
import { deepStrictEqual } from "node:assert/strict";
import { runInNewContext } from "node:vm";
import { expect, it } from "vitest";
import { compactGrammarLiterals } from "./shiki-grammar-literals";

const directory = new URL(
  "../node_modules/@shikijs/langs/dist/",
  import.meta.url,
);
const pinnedGrammars = readdirSync(directory)
  .filter((name) => name.endsWith(".mjs"))
  .flatMap((file) => {
    const source = readFileSync(new URL(file, directory), "utf8");
    if (!source.includes("const lang = Object.freeze(JSON.parse(")) return [];
    const declaration = source.slice(
      source.indexOf("const lang ="),
      source.indexOf("\n\nexport default"),
    );
    return [{ file, declaration }];
  });

it("covers more than 100 pinned grammar modules", () => {
  expect(pinnedGrammars.length).toBeGreaterThan(100);
});

it.each(pinnedGrammars)(
  "preserves $file grammar value and root freezing",
  ({ file, declaration }) => {
    const original = runInNewContext(`${declaration}; lang`);
    const compacted = runInNewContext(
      `${compactGrammarLiterals(declaration)}; lang`,
    );
    // Grammar repositories may own a rule named "constructor"; compare data
    // without treating that property as the object's JavaScript constructor.
    deepStrictEqual(
      structuredClone(compacted),
      structuredClone(original),
      file,
    );
    expect(Object.isFrozen(compacted), file).toBe(true);
  },
);

it("preserves JSON own-property semantics for prototype keys", () => {
  const source = `Object.freeze(JSON.parse(${JSON.stringify('{"nested":{"__proto__":{"x":1}}}')}))`;
  expect(compactGrammarLiterals(source)).toBe(source);
});

it("compacts repeated strings without capturing module bindings or changing grammar keys", () => {
  const repeated = "\\b(?:namespace|template|operator|decltype|requires)\\b";
  const value = {
    [repeated]: repeated,
    patterns: Array.from({ length: 12 }, () => ({
      match: repeated,
      captures: [repeated, true, false, null, 3],
    })),
  };
  const declaration = `const lang = Object.freeze(JSON.parse(${JSON.stringify(JSON.stringify(value))}));`;
  const compacted = compactGrammarLiterals(declaration);
  const result = runInNewContext(
    `const g = "outer binding"; ${compacted}; ({ lang, g })`,
  );
  expect(structuredClone(result.lang)).toStrictEqual(value);
  expect(Object.isFrozen(result.lang)).toBe(true);
  expect(result.g).toBe("outer binding");
  expect(compacted.length).toBeLessThan(
    `const lang = Object.freeze(${JSON.stringify(value)});`.length,
  );
});

it("preserves escaped regex text, Unicode and lone surrogates in repeated strings", () => {
  const text =
    '"quoted" \\b[a-z]+\\b' +
    String.fromCharCode(10, 9, 0x2028, 0x2029) +
    " 中文 😀 " +
    String.fromCharCode(0xd800);
  const value = {
    patterns: [{ match: text }, { begin: text, end: text }],
    aliases: [text, text],
  };
  const source = `Object.freeze(JSON.parse(${JSON.stringify(JSON.stringify(value))}))`;
  const result = runInNewContext(compactGrammarLiterals(source));
  expect(structuredClone(result)).toStrictEqual(value);
  expect(Object.isFrozen(result)).toBe(true);
});

it.each(['{"__proto__":{"x":1}}', '{"patterns":[{"__proto__":{"x":1}}]}'])(
  "retains JSON parsing for prototype keys before repeated-string compaction: %s",
  (json) => {
    const value = JSON.parse(json);
    value.repeated = Array.from(
      { length: 8 },
      () => "a sufficiently long repeated grammar string",
    );
    const source = `Object.freeze(JSON.parse(${JSON.stringify(JSON.stringify(value))}))`;
    expect(compactGrammarLiterals(source)).toBe(source);
  },
);
