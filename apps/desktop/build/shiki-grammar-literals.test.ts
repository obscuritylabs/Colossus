import { readFileSync, readdirSync } from "node:fs";
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
    expect(JSON.stringify(compacted), file).toBe(JSON.stringify(original));
    expect(Object.isFrozen(compacted), file).toBe(true);
  },
);

it("preserves JSON own-property semantics for prototype keys", () => {
  const source = `Object.freeze(JSON.parse(${JSON.stringify('{"nested":{"__proto__":{"x":1}}}')}))`;
  expect(compactGrammarLiterals(source)).toBe(source);
});
