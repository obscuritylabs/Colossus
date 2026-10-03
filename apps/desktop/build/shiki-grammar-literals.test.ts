import { readFileSync, readdirSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { expect, it } from "vitest";
import { compactGrammarLiterals } from "./shiki-grammar-literals";

it("preserves every pinned grammar value and root freezing", () => {
  const directory = new URL(
    "../node_modules/@shikijs/langs/dist/",
    import.meta.url,
  );
  let checked = 0;
  for (const file of readdirSync(directory).filter((name) =>
    name.endsWith(".mjs"),
  )) {
    const source = readFileSync(new URL(file, directory), "utf8");
    if (!source.includes("const lang = Object.freeze(JSON.parse(")) continue;
    const declaration = source.slice(
      source.indexOf("const lang ="),
      source.indexOf("\n\nexport default"),
    );
    const original = runInNewContext(`${declaration}; lang`);
    const compacted = runInNewContext(
      `${compactGrammarLiterals(declaration)}; lang`,
    );
    expect(JSON.stringify(compacted), file).toBe(JSON.stringify(original));
    expect(Object.isFrozen(compacted), file).toBe(true);
    checked++;
  }
  expect(checked).toBeGreaterThan(100);
});

it("preserves JSON own-property semantics for prototype keys", () => {
  const source = `Object.freeze(JSON.parse(${JSON.stringify('{"nested":{"__proto__":{"x":1}}}')}))`;
  expect(compactGrammarLiterals(source)).toBe(source);
});
