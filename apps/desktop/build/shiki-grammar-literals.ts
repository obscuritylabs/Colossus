import type { Plugin } from "vite";

const grammar = /Object\.freeze\(JSON\.parse\(("(?:[^"\\]|\\.)*")\)\)/gu;

function hasPrototypeKey(value: unknown): boolean {
  return (
    value !== null &&
    typeof value === "object" &&
    (Object.hasOwn(value, "__proto__") ||
      Object.values(value).some(hasPrototypeKey))
  );
}

/** Avoid shipping a second layer of JSON escaping in the pinned grammar modules. */
export function compactGrammarLiterals(source: string): string {
  return source.replace(grammar, (original: string, encoded: string) => {
    const value: unknown = JSON.parse(JSON.parse(encoded) as string);
    // An object literal gives __proto__ special semantics; retain JSON.parse for it.
    return hasPrototypeKey(value)
      ? original
      : `Object.freeze(${JSON.stringify(value)})`;
  });
}

export function shikiGrammarLiterals(): Plugin {
  return {
    name: "colossus-shiki-grammar-literals",
    apply: "build",
    enforce: "pre",
    transform(source, id) {
      const path = id.split("?")[0]?.replaceAll("\\", "/") ?? "";
      if (
        !path.includes("/node_modules/@shikijs/langs/dist/") ||
        !path.endsWith(".mjs")
      )
        return null;
      const code = compactGrammarLiterals(source);
      return code === source ? null : { code, map: null };
    },
  };
}
